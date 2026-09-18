//! A minimal, Tokio-native FTP client. Only the commands a downloader needs.

use crate::classify::from_reply;
use osprey_domain::{ErrorKind, IoContext, TaskError};
use std::net::SocketAddr;
use std::pin::Pin;
use std::sync::Arc;
use std::time::Duration;
use tokio::io::{AsyncBufReadExt, AsyncRead, AsyncWrite, AsyncWriteExt, BufReader};
use tokio::net::TcpStream;
use tokio_rustls::TlsConnector;

#[derive(Clone, Debug, PartialEq, Eq)]
pub struct FtpEntry {
    pub name: String,
    pub size: Option<u64>,
    pub is_dir: bool,
    /// Raw modification timestamp (`YYYYMMDDhhmmss` from MLSD/MDTM or the LIST text).
    pub modified: Option<String>,
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Security {
    /// Plain FTP.
    None,
    /// `AUTH TLS` after connecting (FTPES).
    Explicit,
    /// TLS from the first byte (port 990).
    Implicit,
}

pub type BoxStream = Pin<Box<dyn AsyncReadWrite + Send>>;
pub trait AsyncReadWrite: AsyncRead + AsyncWrite + Unpin {}
impl<T: AsyncRead + AsyncWrite + Unpin> AsyncReadWrite for T {}

pub struct Reply {
    pub code: u16,
    pub text: String,
}

pub struct FtpClient {
    control: BufReader<BoxStream>,
    host: String,
    security: Security,
    tls: Option<TlsConnector>,
    server_name: rustls::pki_types::ServerName<'static>,
    pub features: Vec<String>,
    timeout: Duration,
    peer_addr: Option<SocketAddr>,
    epsv_supported: bool,
}

fn tls_connector(verify: bool) -> Result<TlsConnector, TaskError> {
    let provider = Arc::new(rustls::crypto::ring::default_provider());
    let config = if verify {
        use rustls_platform_verifier::BuilderVerifierExt;
        rustls::ClientConfig::builder_with_provider(provider)
            .with_safe_default_protocol_versions()
            .map_err(|e| TaskError::new(ErrorKind::TlsFailure, e.to_string()))?
            .with_platform_verifier()
            .map_err(|e| TaskError::new(ErrorKind::TlsFailure, e.to_string()))?
            .with_no_client_auth()
    } else {
        rustls::ClientConfig::builder_with_provider(provider)
            .with_safe_default_protocol_versions()
            .map_err(|e| TaskError::new(ErrorKind::TlsFailure, e.to_string()))?
            .dangerous()
            .with_custom_certificate_verifier(Arc::new(NoVerify))
            .with_no_client_auth()
    };
    Ok(TlsConnector::from(Arc::new(config)))
}

/// Used only for hosts the user explicitly excepted from verification.
#[derive(Debug)]
struct NoVerify;
impl rustls::client::danger::ServerCertVerifier for NoVerify {
    fn verify_server_cert(
        &self,
        _: &rustls::pki_types::CertificateDer<'_>,
        _: &[rustls::pki_types::CertificateDer<'_>],
        _: &rustls::pki_types::ServerName<'_>,
        _: &[u8],
        _: rustls::pki_types::UnixTime,
    ) -> Result<rustls::client::danger::ServerCertVerified, rustls::Error> {
        Ok(rustls::client::danger::ServerCertVerified::assertion())
    }
    fn verify_tls12_signature(
        &self,
        _: &[u8],
        _: &rustls::pki_types::CertificateDer<'_>,
        _: &rustls::DigitallySignedStruct,
    ) -> Result<rustls::client::danger::HandshakeSignatureValid, rustls::Error> {
        Ok(rustls::client::danger::HandshakeSignatureValid::assertion())
    }
    fn verify_tls13_signature(
        &self,
        _: &[u8],
        _: &rustls::pki_types::CertificateDer<'_>,
        _: &rustls::DigitallySignedStruct,
    ) -> Result<rustls::client::danger::HandshakeSignatureValid, rustls::Error> {
        Ok(rustls::client::danger::HandshakeSignatureValid::assertion())
    }
    fn supported_verify_schemes(&self) -> Vec<rustls::SignatureScheme> {
        rustls::crypto::ring::default_provider()
            .signature_verification_algorithms
            .supported_schemes()
    }
}

fn net_err(e: std::io::Error, ctx: &str) -> TaskError {
    TaskError::from_io_ctx(&e, ctx, IoContext::Network)
}

impl FtpClient {
    /// Connect, negotiate TLS per `security`, and log in.
    pub async fn connect(
        host: &str,
        port: u16,
        security: Security,
        verify_tls: bool,
        user: &str,
        pass: &str,
        timeout: Duration,
    ) -> Result<Self, TaskError> {
        let addr = tokio::net::lookup_host((host, port))
            .await
            .map_err(|e| TaskError::new(ErrorKind::DnsFailure, e.to_string()))?
            .next()
            .ok_or_else(|| {
                TaskError::new(ErrorKind::DnsFailure, format!("no address for {host}"))
            })?;
        let tcp = tokio::time::timeout(timeout, TcpStream::connect(addr))
            .await
            .map_err(|_| TaskError::new(ErrorKind::ConnectionTimeout, "connect timed out"))?
            .map_err(|e| net_err(e, "connect"))?;
        let _ = tcp.set_nodelay(true);
        let server_name = rustls::pki_types::ServerName::try_from(host.to_owned())
            .map_err(|_| TaskError::new(ErrorKind::InvalidUrl, "bad host name"))?;
        let tls = if security == Security::None {
            None
        } else {
            Some(tls_connector(verify_tls)?)
        };
        let stream: BoxStream = match (security, &tls) {
            (Security::Implicit, Some(c)) => Box::pin(
                c.connect(server_name.clone(), tcp)
                    .await
                    .map_err(|e| TaskError::new(ErrorKind::TlsFailure, e.to_string()))?,
            ),
            _ => Box::pin(tcp),
        };
        let mut client = Self {
            control: BufReader::new(stream),
            host: host.to_owned(),
            security,
            tls,
            server_name,
            features: Vec::new(),
            timeout,
            peer_addr: Some(addr),
            epsv_supported: true,
        };
        let welcome = client.read_reply().await?;
        if welcome.code != 220 {
            return Err(from_reply(welcome.code, &welcome.text, "welcome"));
        }
        if security == Security::Explicit {
            let r = client.command("AUTH TLS").await?;
            if r.code != 234 && r.code != 334 {
                return Err(from_reply(r.code, &r.text, "AUTH TLS"));
            }
            client.upgrade_control().await?;
        }
        client.login(user, pass).await?;
        if security != Security::None {
            let _ = client.command("PBSZ 0").await?;
            let r = client.command("PROT P").await?;
            if r.code / 100 != 2 {
                return Err(from_reply(r.code, &r.text, "PROT P"));
            }
        }
        if let Ok(r) = client.command("FEAT").await {
            if r.code == 211 {
                client.features = r
                    .text
                    .lines()
                    .skip(1)
                    .map(|l| l.trim().to_ascii_uppercase())
                    .filter(|l| !l.starts_with("211"))
                    .collect();
            }
        }
        let r = client.command("TYPE I").await?;
        if r.code / 100 != 2 {
            return Err(from_reply(r.code, &r.text, "TYPE I"));
        }
        Ok(client)
    }

    async fn upgrade_control(&mut self) -> Result<(), TaskError> {
        let Some(tls) = &self.tls else { return Ok(()) };
        // take the plain stream out of the reader (no buffered bytes remain after a full reply)
        let plain = std::mem::replace(
            &mut self.control,
            BufReader::new(Box::pin(tokio::io::empty()) as BoxStream),
        )
        .into_inner();
        let secured = tls
            .connect(self.server_name.clone(), plain)
            .await
            .map_err(|e| TaskError::new(ErrorKind::TlsFailure, e.to_string()))?;
        self.control = BufReader::new(Box::pin(secured));
        Ok(())
    }

    async fn login(&mut self, user: &str, pass: &str) -> Result<(), TaskError> {
        let r = self.command(&format!("USER {user}")).await?;
        match r.code {
            230 => Ok(()),
            331 | 332 => {
                let r = self.command(&format!("PASS {pass}")).await?;
                if r.code / 100 == 2 {
                    Ok(())
                } else {
                    Err(from_reply(r.code, &r.text, "login"))
                }
            }
            _ => Err(from_reply(r.code, &r.text, "login")),
        }
    }

    pub fn supports(&self, feature: &str) -> bool {
        self.features
            .iter()
            .any(|f| f.starts_with(&feature.to_ascii_uppercase()))
    }

    pub async fn command(&mut self, cmd: &str) -> Result<Reply, TaskError> {
        if cmd.contains('\r') || cmd.contains('\n') {
            return Err(TaskError::new(
                ErrorKind::InvalidUrl,
                "control characters in FTP command",
            ));
        }
        let line = format!("{cmd}\r\n");
        tokio::time::timeout(
            self.timeout,
            self.control.get_mut().write_all(line.as_bytes()),
        )
        .await
        .map_err(|_| TaskError::new(ErrorKind::ReadTimeout, "control write timed out"))?
        .map_err(|e| net_err(e, "control write"))?;
        self.read_reply().await
    }

    pub async fn read_reply(&mut self) -> Result<Reply, TaskError> {
        let mut text = String::new();
        let mut code: Option<u16> = None;
        loop {
            let mut line = String::new();
            let n = tokio::time::timeout(self.timeout, self.control.read_line(&mut line))
                .await
                .map_err(|_| TaskError::new(ErrorKind::ReadTimeout, "control read timed out"))?
                .map_err(|e| net_err(e, "control read"))?;
            if n == 0 {
                return Err(TaskError::new(
                    ErrorKind::ConnectionReset,
                    "control connection closed",
                ));
            }
            if text.len() + line.len() > 64 * 1024 {
                return Err(TaskError::new(ErrorKind::ParseError, "FTP reply too long"));
            }
            text.push_str(&line);
            let bytes = line.as_bytes();
            if bytes.len() >= 4 && bytes[..3].iter().all(u8::is_ascii_digit) {
                let c: u16 = line[..3].parse().unwrap_or(0);
                match code {
                    None if bytes[3] == b'-' => code = Some(c),
                    None => return Ok(Reply { code: c, text }),
                    Some(first) if first == c && bytes[3] == b' ' => {
                        return Ok(Reply { code: c, text })
                    }
                    _ => {}
                }
            } else if code.is_none() {
                return Err(TaskError::new(
                    ErrorKind::ParseError,
                    format!("malformed FTP reply: {}", line.trim()),
                ));
            }
        }
    }

    pub async fn size(&mut self, path: &str) -> Result<Option<u64>, TaskError> {
        let r = self.command(&format!("SIZE {path}")).await?;
        match r.code {
            213 => Ok(r.text[4..].trim().parse().ok()),
            550 => Err(from_reply(r.code, &r.text, "SIZE")),
            _ => Ok(None),
        }
    }

    pub async fn mdtm(&mut self, path: &str) -> Result<Option<String>, TaskError> {
        let r = self.command(&format!("MDTM {path}")).await?;
        Ok(if r.code == 213 {
            Some(r.text[4..].trim().to_owned())
        } else {
            None
        })
    }

    /// Open a passive data connection (EPSV, falling back to PASV).
    async fn open_data(&mut self) -> Result<BoxStream, TaskError> {
        let addr = if self.epsv_supported {
            match self.command("EPSV").await {
                Ok(r) if r.code == 229 => {
                    let port = parse_epsv(&r.text)
                        .ok_or_else(|| TaskError::new(ErrorKind::ParseError, "bad EPSV reply"))?;
                    let ip = self
                        .peer_addr
                        .map(|a| a.ip())
                        .ok_or_else(|| TaskError::internal("no peer address"))?;
                    Some(SocketAddr::new(ip, port))
                }
                _ => {
                    self.epsv_supported = false;
                    None
                }
            }
        } else {
            None
        };
        let addr = match addr {
            Some(a) => a,
            None => {
                let r = self.command("PASV").await?;
                if r.code != 227 {
                    return Err(from_reply(r.code, &r.text, "PASV"));
                }
                let (mut ip, port) = parse_pasv(&r.text)
                    .ok_or_else(|| TaskError::new(ErrorKind::ParseError, "bad PASV reply"))?;
                // NAT workaround: servers behind NAT announce private addresses; use the control peer.
                if let Some(peer) = self.peer_addr {
                    if osprey_runtime::net::is_private_ip(ip)
                        && !osprey_runtime::net::is_private_ip(peer.ip())
                    {
                        ip = peer.ip();
                    }
                }
                SocketAddr::new(ip, port)
            }
        };
        let tcp = tokio::time::timeout(self.timeout, TcpStream::connect(addr))
            .await
            .map_err(|_| TaskError::new(ErrorKind::ConnectionTimeout, "data connect timed out"))?
            .map_err(|e| net_err(e, "data connect"))?;
        match &self.tls {
            Some(c) if self.security != Security::None => Ok(Box::pin(
                c.connect(self.server_name.clone(), tcp)
                    .await
                    .map_err(|e| TaskError::new(ErrorKind::TlsFailure, e.to_string()))?,
            )),
            _ => Ok(Box::pin(tcp)),
        }
    }

    /// Start `RETR path` from `offset`; returns the data stream. Call [`Self::finish_transfer`]
    /// after reading it to completion, or [`Self::abort`] to stop early.
    pub async fn retr(&mut self, path: &str, offset: u64) -> Result<BoxStream, TaskError> {
        if offset > 0 {
            let r = self.command(&format!("REST {offset}")).await?;
            if r.code != 350 {
                return Err(TaskError::new(
                    ErrorKind::RangeNotSupported,
                    format!("REST refused: {} {}", r.code, r.text.trim()),
                )
                .with_status(r.code));
            }
        }
        let data = self.open_data().await?;
        let r = self.command(&format!("RETR {path}")).await?;
        if r.code / 100 != 1 {
            return Err(from_reply(r.code, &r.text, "RETR"));
        }
        Ok(data)
    }

    /// Read the final 226/250 after the data stream reached EOF.
    pub async fn finish_transfer(&mut self) -> Result<(), TaskError> {
        let r = self.read_reply().await?;
        if r.code / 100 == 2 {
            Ok(())
        } else {
            Err(from_reply(r.code, &r.text, "transfer end"))
        }
    }

    /// Best-effort abort of an in-progress transfer.
    pub async fn abort(&mut self) {
        let _ = tokio::time::timeout(Duration::from_secs(3), async {
            let _ = self.control.get_mut().write_all(b"ABOR\r\n").await;
            let _ = self.read_reply().await;
            let _ = self.read_reply().await;
        })
        .await;
    }

    /// Directory listing via MLSD when supported, else LIST.
    pub async fn list(&mut self, path: &str) -> Result<Vec<FtpEntry>, TaskError> {
        let use_mlsd = self.supports("MLSD");
        let data = self.open_data().await?;
        let cmd = if use_mlsd {
            format!("MLSD {path}")
        } else {
            format!("LIST {path}")
        };
        let r = self.command(&cmd).await?;
        if r.code / 100 != 1 {
            return Err(from_reply(r.code, &r.text, "LIST"));
        }
        let mut reader = BufReader::new(data);
        let mut entries = Vec::new();
        let mut line = String::new();
        loop {
            line.clear();
            let n = tokio::time::timeout(self.timeout, reader.read_line(&mut line))
                .await
                .map_err(|_| TaskError::new(ErrorKind::ReadTimeout, "listing timed out"))?
                .map_err(|e| net_err(e, "listing"))?;
            if n == 0 {
                break;
            }
            let l = line.trim_end_matches(['\r', '\n']);
            let parsed = if use_mlsd {
                crate::listing::parse_mlsd_line(l)
            } else {
                crate::listing::parse_list_line(l)
            };
            if let Some(e) = parsed {
                entries.push(e);
            }
            if entries.len() > 50_000 {
                break;
            }
        }
        drop(reader);
        self.finish_transfer().await?;
        Ok(entries)
    }

    pub async fn quit(mut self) {
        let _ = tokio::time::timeout(Duration::from_secs(2), self.command("QUIT")).await;
    }

    pub fn host(&self) -> &str {
        &self.host
    }
}

fn parse_epsv(text: &str) -> Option<u16> {
    // 229 Entering Extended Passive Mode (|||6446|)
    let start = text.find("(|||")? + 4;
    let end = text[start..].find('|')? + start;
    text[start..end].parse().ok()
}

fn parse_pasv(text: &str) -> Option<(std::net::IpAddr, u16)> {
    // 227 Entering Passive Mode (192,168,1,2,19,203)
    let start = text.find('(')? + 1;
    let end = text[start..].find(')')? + start;
    let nums: Vec<u16> = text[start..end]
        .split(',')
        .map(|s| s.trim().parse().ok())
        .collect::<Option<Vec<u16>>>()?;
    if nums.len() != 6 {
        return None;
    }
    let ip = std::net::Ipv4Addr::new(nums[0] as u8, nums[1] as u8, nums[2] as u8, nums[3] as u8);
    Some((ip.into(), nums[4] * 256 + nums[5]))
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn parses_passive_replies() {
        assert_eq!(
            parse_epsv("229 Entering Extended Passive Mode (|||6446|)\r\n"),
            Some(6446)
        );
        assert_eq!(
            parse_pasv("227 Entering Passive Mode (192,168,1,2,19,203).\r\n"),
            Some(("192.168.1.2".parse().unwrap(), 19 * 256 + 203))
        );
        assert!(parse_pasv("227 nope").is_none());
    }
}
