//! A minimal in-process FTP server for tests: passive mode only, in-memory files, optional
//! "drop the data connection after N bytes on the first RETR" fault injection.

use std::collections::HashMap;
use std::net::SocketAddr;
use std::sync::atomic::{AtomicU64, AtomicUsize, Ordering};
use std::sync::Arc;
use tokio::io::{AsyncBufReadExt, AsyncWriteExt, BufReader};
use tokio::net::{TcpListener, TcpStream};

pub struct FtpServer {
    pub addr: SocketAddr,
    pub files: Arc<HashMap<String, Vec<u8>>>,
    pub retr_count: Arc<AtomicUsize>,
    pub fail_first_after: Arc<AtomicU64>,
    pub rest_offsets: Arc<parking::Seq>,
    pub deny_rest: bool,
}

pub mod parking {
    use std::sync::Mutex;
    #[derive(Default)]
    pub struct Seq(Mutex<Vec<u64>>);
    impl Seq {
        pub fn push(&self, v: u64) {
            self.0.lock().unwrap().push(v);
        }
        pub fn get(&self) -> Vec<u64> {
            self.0.lock().unwrap().clone()
        }
    }
}

impl FtpServer {
    pub async fn start(
        files: HashMap<String, Vec<u8>>,
        fail_first_after: Option<u64>,
        deny_rest: bool,
    ) -> Self {
        let listener = TcpListener::bind("127.0.0.1:0").await.unwrap();
        let addr = listener.local_addr().unwrap();
        let files = Arc::new(files);
        let retr_count = Arc::new(AtomicUsize::new(0));
        let fail = Arc::new(AtomicU64::new(fail_first_after.unwrap_or(0)));
        let rests = Arc::new(parking::Seq::default());
        let (f, rc, fa, ro) = (
            files.clone(),
            retr_count.clone(),
            fail.clone(),
            rests.clone(),
        );
        tokio::spawn(async move {
            loop {
                let Ok((sock, _)) = listener.accept().await else {
                    break;
                };
                let (f, rc, fa, ro) = (f.clone(), rc.clone(), fa.clone(), ro.clone());
                tokio::spawn(async move {
                    let _ = session(sock, f, rc, fa, ro, deny_rest).await;
                });
            }
        });
        Self {
            addr,
            files,
            retr_count,
            fail_first_after: fail,
            rest_offsets: rests,
            deny_rest,
        }
    }

    pub fn url(&self, path: &str, user: Option<(&str, &str)>) -> String {
        match user {
            Some((u, p)) => format!("ftp://{u}:{p}@{}{}", self.addr, path),
            None => format!("ftp://{}{}", self.addr, path),
        }
    }
}

async fn session(
    sock: TcpStream,
    files: Arc<HashMap<String, Vec<u8>>>,
    retr_count: Arc<AtomicUsize>,
    fail_after: Arc<AtomicU64>,
    rests: Arc<parking::Seq>,
    deny_rest: bool,
) -> std::io::Result<()> {
    let (r, mut w) = sock.into_split();
    let mut r = BufReader::new(r);
    w.write_all(b"220 test ftpd ready\r\n").await?;
    let mut user = String::new();
    let mut logged_in = false;
    let mut rest: u64 = 0;
    let mut pasv: Option<TcpListener> = None;
    let mut line = String::new();
    loop {
        line.clear();
        if r.read_line(&mut line).await? == 0 {
            return Ok(());
        }
        let l = line.trim_end();
        let (cmd, arg) = match l.split_once(' ') {
            Some((c, a)) => (c.to_ascii_uppercase(), a.to_owned()),
            None => (l.to_ascii_uppercase(), String::new()),
        };
        match cmd.as_str() {
            "USER" => {
                user = arg.clone();
                if user == "anonymous" {
                    logged_in = true;
                    w.write_all(b"230 anonymous ok\r\n").await?;
                } else {
                    w.write_all(b"331 password required\r\n").await?;
                }
            }
            "PASS" => {
                if user == "u" && arg == "p" {
                    logged_in = true;
                    w.write_all(b"230 logged in\r\n").await?;
                } else {
                    w.write_all(b"530 login incorrect\r\n").await?;
                }
            }
            "FEAT" => {
                let mut f =
                    String::from("211-Features:\r\n SIZE\r\n MDTM\r\n MLSD\r\n EPSV\r\n UTF8\r\n");
                if !deny_rest {
                    f.push_str(" REST STREAM\r\n");
                }
                f.push_str("211 End\r\n");
                w.write_all(f.as_bytes()).await?;
            }
            "TYPE" => w.write_all(b"200 type set\r\n").await?,
            "PBSZ" | "PROT" => w.write_all(b"200 ok\r\n").await?,
            "AUTH" => w.write_all(b"502 not supported\r\n").await?,
            "SIZE" => match files.get(arg.as_str()) {
                Some(f) if logged_in => {
                    w.write_all(format!("213 {}\r\n", f.len()).as_bytes())
                        .await?
                }
                _ => w.write_all(b"550 no such file\r\n").await?,
            },
            "MDTM" => match files.get(arg.as_str()) {
                Some(_) => w.write_all(b"213 20250101120000\r\n").await?,
                None => w.write_all(b"550 no such file\r\n").await?,
            },
            "EPSV" => {
                let l = TcpListener::bind("127.0.0.1:0").await?;
                let port = l.local_addr()?.port();
                pasv = Some(l);
                w.write_all(
                    format!("229 Entering Extended Passive Mode (|||{port}|)\r\n").as_bytes(),
                )
                .await?;
            }
            "PASV" => {
                let l = TcpListener::bind("127.0.0.1:0").await?;
                let port = l.local_addr()?.port();
                pasv = Some(l);
                w.write_all(
                    format!(
                        "227 Entering Passive Mode (127,0,0,1,{},{})\r\n",
                        port / 256,
                        port % 256
                    )
                    .as_bytes(),
                )
                .await?;
            }
            "REST" => {
                if deny_rest {
                    w.write_all(b"502 REST not implemented\r\n").await?;
                } else {
                    rest = arg.parse().unwrap_or(0);
                    rests.push(rest);
                    w.write_all(b"350 restarting\r\n").await?;
                }
            }
            "RETR" => {
                let Some(l) = pasv.take() else {
                    w.write_all(b"425 use PASV first\r\n").await?;
                    continue;
                };
                match files.get(arg.as_str()) {
                    Some(f) if logged_in => {
                        w.write_all(b"150 opening data connection\r\n").await?;
                        let (mut data, _) = l.accept().await?;
                        let n = retr_count.fetch_add(1, Ordering::SeqCst);
                        let start = (rest as usize).min(f.len());
                        rest = 0;
                        let body = &f[start..];
                        let fail = fail_after.load(Ordering::SeqCst);
                        if n == 0 && fail > 0 && (fail as usize) < body.len() {
                            data.write_all(&body[..fail as usize]).await?;
                            let _ = data.flush().await;
                            drop(data);
                            w.write_all(b"426 connection closed; transfer aborted\r\n")
                                .await?;
                        } else {
                            for chunk in body.chunks(16 * 1024) {
                                data.write_all(chunk).await?;
                            }
                            data.shutdown().await?;
                            drop(data);
                            w.write_all(b"226 transfer complete\r\n").await?;
                        }
                    }
                    _ => w.write_all(b"550 no such file\r\n").await?,
                }
            }
            "MLSD" | "LIST" => {
                let Some(l) = pasv.take() else {
                    w.write_all(b"425 use PASV first\r\n").await?;
                    continue;
                };
                w.write_all(b"150 listing\r\n").await?;
                let (mut data, _) = l.accept().await?;
                let mut out = String::new();
                for (name, body) in files.iter() {
                    if cmd == "MLSD" {
                        out.push_str(&format!(
                            "type=file;size={};modify=20250101120000; {}\r\n",
                            body.len(),
                            name.trim_start_matches('/')
                        ));
                    } else {
                        out.push_str(&format!(
                            "-rw-r--r--   1 ftp ftp {:>10} Jan  1 12:00 {}\r\n",
                            body.len(),
                            name.trim_start_matches('/')
                        ));
                    }
                }
                data.write_all(out.as_bytes()).await?;
                data.shutdown().await?;
                drop(data);
                w.write_all(b"226 done\r\n").await?;
            }
            "ABOR" => w.write_all(b"226 aborted\r\n").await?,
            "QUIT" => {
                w.write_all(b"221 bye\r\n").await?;
                return Ok(());
            }
            "NOOP" => w.write_all(b"200 ok\r\n").await?,
            _ => w.write_all(b"502 command not implemented\r\n").await?,
        }
    }
}
