//! Ed25519 release signatures.
//!
//! A release's `Swoop-X.Y.Z.dmg.sig` asset is one line of base64: the 64-byte Ed25519 signature
//! over the 32-byte SHA-256 digest of the DMG. The private key file is one line of hex (the
//! 32-byte seed); it lives outside the repository, `chmod 600`.

use base64::Engine as _;
use ed25519_dalek::{Signature, Signer, SigningKey, Verifier, VerifyingKey};
use sha2::{Digest, Sha256};
use std::io::Read;
use std::path::Path;
use swoop_domain::{ErrorKind, TaskError};

/// Decode a hex Ed25519 public key.
pub fn parse_public_key(hex_key: &str) -> Result<VerifyingKey, TaskError> {
    let bytes = hex::decode(hex_key.trim())
        .map_err(|e| TaskError::new(ErrorKind::Internal, format!("bad update key: {e}")))?;
    let arr: [u8; 32] = bytes
        .try_into()
        .map_err(|_| TaskError::new(ErrorKind::Internal, "update key must be 32 bytes"))?;
    VerifyingKey::from_bytes(&arr)
        .map_err(|e| TaskError::new(ErrorKind::Internal, format!("bad update key: {e}")))
}

/// Decode the contents of a `.sig` asset (base64, surrounding whitespace ignored).
pub fn parse_signature(text: &str) -> Result<Signature, TaskError> {
    let bytes = base64::engine::general_purpose::STANDARD
        .decode(text.trim())
        .map_err(|_| TaskError::new(ErrorKind::ParseError, "update signature isn't base64"))?;
    Signature::from_slice(&bytes).map_err(|_| {
        TaskError::new(
            ErrorKind::ParseError,
            "update signature has the wrong length",
        )
    })
}

/// Check a signature over a SHA-256 digest.
pub fn verify_digest(
    key: &VerifyingKey,
    digest: &[u8; 32],
    sig: &Signature,
) -> Result<(), TaskError> {
    key.verify(digest, sig).map_err(|_| {
        TaskError::new(
            ErrorKind::CertificateInvalid,
            "the update's signature doesn't match Swoop's release key",
        )
    })
}

/// SHA-256 of a file, streamed.
pub fn sha256_file(path: &Path) -> std::io::Result<[u8; 32]> {
    let mut f = std::fs::File::open(path)?;
    let mut h = Sha256::new();
    let mut buf = vec![0u8; 256 * 1024];
    loop {
        let n = f.read(&mut buf)?;
        if n == 0 {
            break;
        }
        h.update(&buf[..n]);
    }
    Ok(h.finalize().into())
}

/// Hash `path` and check `sig_text` (a `.sig` asset's contents) against it.
pub fn verify_file(key: &VerifyingKey, path: &Path, sig_text: &str) -> Result<(), TaskError> {
    let sig = parse_signature(sig_text)?;
    let digest = sha256_file(path).map_err(|e| TaskError::from_io(&e, "hash update"))?;
    verify_digest(key, &digest, &sig)
}

/// Sign a file: base64 Ed25519 signature over its SHA-256.
pub fn sign_file(key: &SigningKey, path: &Path) -> std::io::Result<String> {
    let digest = sha256_file(path)?;
    Ok(base64::engine::general_purpose::STANDARD.encode(key.sign(&digest).to_bytes()))
}

pub fn public_key_hex(key: &SigningKey) -> String {
    hex::encode(key.verifying_key().to_bytes())
}

/// Read a private key file (hex seed).
pub fn load_signing_key(path: &Path) -> Result<SigningKey, String> {
    let text = std::fs::read_to_string(path)
        .map_err(|e| format!("can't read signing key {}: {e}", path.display()))?;
    let bytes = hex::decode(text.trim()).map_err(|_| "signing key isn't hex".to_owned())?;
    let seed: [u8; 32] = bytes
        .try_into()
        .map_err(|_| "signing key must be 32 bytes".to_owned())?;
    Ok(SigningKey::from_bytes(&seed))
}

/// Create a new private key at `path` (directory 700, file 600) and return the public key hex.
/// Refuses to overwrite an existing key: losing it strands every install.
pub fn generate_signing_key(path: &Path) -> Result<String, String> {
    use rand::RngCore;
    if path.exists() {
        return Err(format!(
            "{} already exists; refusing to overwrite a release key",
            path.display()
        ));
    }
    if let Some(dir) = path.parent() {
        std::fs::create_dir_all(dir).map_err(|e| format!("create {}: {e}", dir.display()))?;
        #[cfg(unix)]
        {
            use std::os::unix::fs::PermissionsExt;
            std::fs::set_permissions(dir, std::fs::Permissions::from_mode(0o700))
                .map_err(|e| format!("chmod {}: {e}", dir.display()))?;
        }
    }
    let mut seed = [0u8; 32];
    rand::rngs::OsRng.fill_bytes(&mut seed);
    let key = SigningKey::from_bytes(&seed);
    let mut opts = std::fs::OpenOptions::new();
    opts.write(true).create_new(true);
    #[cfg(unix)]
    {
        use std::os::unix::fs::OpenOptionsExt;
        opts.mode(0o600);
    }
    let mut f = opts
        .open(path)
        .map_err(|e| format!("create {}: {e}", path.display()))?;
    use std::io::Write;
    writeln!(f, "{}", hex::encode(seed)).map_err(|e| format!("write key: {e}"))?;
    f.sync_all().map_err(|e| format!("sync key: {e}"))?;
    Ok(public_key_hex(&key))
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn sign_verify_and_tamper() {
        let dir = tempfile::tempdir().unwrap();
        let key_path = dir.path().join("keys/update-signing-key");
        let pk_hex = generate_signing_key(&key_path).unwrap();
        #[cfg(unix)]
        {
            use std::os::unix::fs::PermissionsExt;
            let mode = std::fs::metadata(&key_path).unwrap().permissions().mode();
            assert_eq!(mode & 0o777, 0o600);
        }
        assert!(
            generate_signing_key(&key_path).is_err(),
            "never overwrite a key"
        );
        let sk = load_signing_key(&key_path).unwrap();
        assert_eq!(public_key_hex(&sk), pk_hex);
        let pk = parse_public_key(&pk_hex).unwrap();

        let dmg = dir.path().join("Swoop-9.9.9.dmg");
        std::fs::write(&dmg, vec![42u8; 300_000]).unwrap();
        let sig = sign_file(&sk, &dmg).unwrap();
        verify_file(&pk, &dmg, &format!("{sig}\n")).unwrap();

        // one flipped byte → refused
        let mut bytes = std::fs::read(&dmg).unwrap();
        bytes[150_000] ^= 1;
        std::fs::write(&dmg, &bytes).unwrap();
        assert_eq!(
            verify_file(&pk, &dmg, &sig).unwrap_err().kind,
            ErrorKind::CertificateInvalid
        );

        // right file, someone else's key → refused
        bytes[150_000] ^= 1;
        std::fs::write(&dmg, &bytes).unwrap();
        let other = SigningKey::from_bytes(&[9u8; 32]);
        let forged = sign_file(&other, &dmg).unwrap();
        assert!(verify_file(&pk, &dmg, &forged).is_err());
        // garbage signature text → refused
        assert!(verify_file(&pk, &dmg, "not base64!").is_err());
        assert!(verify_file(&pk, &dmg, "AAAA").is_err());
    }
}
