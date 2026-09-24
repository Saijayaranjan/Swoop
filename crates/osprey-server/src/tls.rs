//! Self-signed certificate for the remote listener, persisted as `cert.pem` / `key.pem` (0600)
//! so paired devices can pin its SHA-256 fingerprint across restarts.

use osprey_domain::{DomainError, DomainResult};
use rustls::pki_types::pem::PemObject;
use rustls::pki_types::{CertificateDer, PrivateKeyDer};
use sha2::{Digest, Sha256};
use std::io::Write;
use std::net::IpAddr;
use std::path::Path;
use std::sync::Arc;

pub(crate) struct TlsMaterial {
    pub cert_der: Vec<u8>,
    pub key_der: Vec<u8>,
    pub fingerprint: String,
}

pub(crate) fn fingerprint(cert_der: &[u8]) -> String {
    hex::encode(Sha256::digest(cert_der))
}

fn io_err(what: &str, e: impl std::fmt::Display) -> DomainError {
    DomainError::Storage(format!("{what}: {e}"))
}

/// Load `cert.pem`/`key.pem` from `dir`, or generate and persist a new pair.
pub(crate) fn load_or_create(dir: &Path, bind: IpAddr) -> DomainResult<TlsMaterial> {
    let cert_path = dir.join("cert.pem");
    let key_path = dir.join("key.pem");
    if cert_path.exists() && key_path.exists() {
        match load(&cert_path, &key_path) {
            Ok(m) => return Ok(m),
            Err(e) => {
                tracing::warn!(error = %e, "existing TLS certificate unusable; generating a new one")
            }
        }
    }
    create_private_dir(dir)?;

    let mut sans = vec!["localhost".to_owned(), "127.0.0.1".to_owned(), "::1".to_owned()];
    if !bind.is_unspecified() && !bind.is_loopback() {
        sans.push(bind.to_string());
    }
    let rcgen::CertifiedKey { cert, key_pair } =
        rcgen::generate_simple_self_signed(sans).map_err(|e| io_err("generate certificate", e))?;
    write_file(&key_path, key_pair.serialize_pem().as_bytes(), 0o600)?;
    write_file(&cert_path, cert.pem().as_bytes(), 0o644)?;
    let cert_der = cert.der().to_vec();
    Ok(TlsMaterial {
        fingerprint: fingerprint(&cert_der),
        cert_der,
        key_der: key_pair.serialize_der(),
    })
}

fn load(cert_path: &Path, key_path: &Path) -> Result<TlsMaterial, String> {
    let cert = CertificateDer::from_pem_file(cert_path).map_err(|e| e.to_string())?;
    let key = PrivateKeyDer::from_pem_file(key_path).map_err(|e| e.to_string())?;
    let cert_der = cert.as_ref().to_vec();
    let key_der = key.secret_der().to_vec();
    // Validate the pair actually forms a usable config.
    let m = TlsMaterial {
        fingerprint: fingerprint(&cert_der),
        cert_der,
        key_der,
    };
    server_config(&m).map_err(|e| e.to_string())?;
    Ok(m)
}

fn create_private_dir(dir: &Path) -> DomainResult<()> {
    std::fs::create_dir_all(dir).map_err(|e| io_err("create TLS directory", e))?;
    #[cfg(unix)]
    {
        use std::os::unix::fs::PermissionsExt;
        let _ = std::fs::set_permissions(dir, std::fs::Permissions::from_mode(0o700));
    }
    Ok(())
}

fn write_file(path: &Path, bytes: &[u8], mode: u32) -> DomainResult<()> {
    let tmp = path.with_extension("tmp");
    let _ = std::fs::remove_file(&tmp);
    let mut opts = std::fs::OpenOptions::new();
    opts.write(true).create_new(true);
    #[cfg(unix)]
    {
        use std::os::unix::fs::OpenOptionsExt;
        opts.mode(mode);
    }
    #[cfg(not(unix))]
    let _ = mode;
    let mut f = opts
        .open(&tmp)
        .map_err(|e| io_err(&format!("write {}", tmp.display()), e))?;
    f.write_all(bytes)
        .and_then(|_| f.sync_all())
        .map_err(|e| io_err("write TLS file", e))?;
    std::fs::rename(&tmp, path).map_err(|e| io_err("install TLS file", e))?;
    Ok(())
}

/// rustls server config with an explicit crypto provider (the process default is ambiguous in
/// this workspace because both `ring` and `aws-lc-rs` are compiled in).
pub(crate) fn server_config(m: &TlsMaterial) -> DomainResult<rustls::ServerConfig> {
    let provider = Arc::new(rustls::crypto::ring::default_provider());
    let key = PrivateKeyDer::try_from(m.key_der.clone())
        .map_err(|e| DomainError::Internal(format!("TLS key: {e}")))?;
    let mut cfg = rustls::ServerConfig::builder_with_provider(provider)
        .with_safe_default_protocol_versions()
        .map_err(|e| DomainError::Internal(format!("TLS config: {e}")))?
        .with_no_client_auth()
        .with_single_cert(vec![CertificateDer::from(m.cert_der.clone())], key)
        .map_err(|e| DomainError::Internal(format!("TLS certificate: {e}")))?;
    cfg.alpn_protocols = vec![b"h2".to_vec(), b"http/1.1".to_vec()];
    Ok(cfg)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn creates_then_reuses() {
        let dir = tempfile::tempdir().unwrap();
        let tls = dir.path().join("tls");
        let a = load_or_create(&tls, IpAddr::from([0, 0, 0, 0])).unwrap();
        let b = load_or_create(&tls, IpAddr::from([0, 0, 0, 0])).unwrap();
        assert_eq!(a.fingerprint, b.fingerprint);
        assert_eq!(a.fingerprint.len(), 64);
        #[cfg(unix)]
        {
            use std::os::unix::fs::PermissionsExt;
            let mode = std::fs::metadata(tls.join("key.pem")).unwrap().permissions().mode();
            assert_eq!(mode & 0o777, 0o600);
        }
    }
}
