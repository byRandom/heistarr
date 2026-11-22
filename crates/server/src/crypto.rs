//! Encryption of stored secrets (API keys, passwords) with a master key.
//!
//! The key comes from `HEISTARR_MASTER_KEY` (base64 of 32 bytes) or from
//! `master.key` in the data folder, created on first start with mode 0600.
//! Losing the key means re-entering the secrets; everything else survives.

use anyhow::{Context, Result, bail};
use base64::Engine;
use base64::engine::general_purpose::STANDARD as B64;
use chacha20poly1305::aead::{Aead, KeyInit};
use chacha20poly1305::{XChaCha20Poly1305, XNonce};
use rand::RngCore;
use std::path::Path;
use std::sync::Arc;

const PREFIX: &str = "enc:v1:";

#[derive(Clone)]
pub struct Cipher(Arc<XChaCha20Poly1305>);

impl Cipher {
    pub fn from_key(key: &[u8; 32]) -> Self {
        Self(Arc::new(XChaCha20Poly1305::new(key.into())))
    }

    /// A throwaway key, for tests.
    pub fn random() -> Self {
        let mut k = [0u8; 32];
        rand::rng().fill_bytes(&mut k);
        Self::from_key(&k)
    }

    /// Loads the master key from the environment or the data folder,
    /// creating it in the data folder if neither exists.
    pub fn load(data_dir: &Path) -> Result<Self> {
        if let Ok(v) = std::env::var("HEISTARR_MASTER_KEY") {
            return Self::from_b64(v.trim()).context("HEISTARR_MASTER_KEY");
        }
        let path = data_dir.join("master.key");
        if path.exists() {
            let v = std::fs::read_to_string(&path)
                .with_context(|| format!("reading {}", path.display()))?;
            return Self::from_b64(v.trim()).with_context(|| format!("{}", path.display()));
        }
        let mut k = [0u8; 32];
        rand::rng().fill_bytes(&mut k);
        write_private(&path, B64.encode(k).as_bytes())?;
        tracing::info!("created master key at {}", path.display());
        Ok(Self::from_key(&k))
    }

    fn from_b64(v: &str) -> Result<Self> {
        let bytes = B64.decode(v).context("not base64")?;
        let Ok(k) = <[u8; 32]>::try_from(bytes.as_slice()) else {
            bail!("the master key must be 32 bytes");
        };
        Ok(Self::from_key(&k))
    }

    pub fn encrypt(&self, plain: &str) -> String {
        if plain.is_empty() {
            return String::new();
        }
        let mut nonce = [0u8; 24];
        rand::rng().fill_bytes(&mut nonce);
        let ct = self
            .0
            .encrypt(XNonce::from_slice(&nonce), plain.as_bytes())
            .expect("encryption cannot fail");
        let mut out = nonce.to_vec();
        out.extend_from_slice(&ct);
        format!("{PREFIX}{}", B64.encode(out))
    }

    /// Decrypts a stored value. Values without the prefix are returned as
    /// they are (empty values, or ones stored before encryption).
    pub fn decrypt(&self, stored: &str) -> Result<String> {
        let Some(b) = stored.strip_prefix(PREFIX) else {
            return Ok(stored.to_string());
        };
        let raw = B64.decode(b).context("corrupt secret")?;
        if raw.len() < 24 {
            bail!("corrupt secret");
        }
        let (nonce, ct) = raw.split_at(24);
        let plain = self.0.decrypt(XNonce::from_slice(nonce), ct).map_err(|_| {
            anyhow::anyhow!("cannot decrypt a stored secret: was the master key changed?")
        })?;
        Ok(String::from_utf8(plain)?)
    }
}

fn write_private(path: &Path, bytes: &[u8]) -> Result<()> {
    use std::io::Write;
    let mut o = std::fs::OpenOptions::new();
    o.write(true).create_new(true);
    #[cfg(unix)]
    {
        use std::os::unix::fs::OpenOptionsExt;
        o.mode(0o600);
    }
    let mut f = o
        .open(path)
        .with_context(|| format!("creating {}", path.display()))?;
    f.write_all(bytes)?;
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn roundtrip_and_wrong_key() {
        let a = Cipher::random();
        let e = a.encrypt("hunter2");
        assert!(e.starts_with(PREFIX));
        assert_ne!(e, a.encrypt("hunter2"), "fresh nonce every time");
        assert_eq!(a.decrypt(&e).unwrap(), "hunter2");
        assert!(Cipher::random().decrypt(&e).is_err());
        assert_eq!(a.decrypt("plain").unwrap(), "plain");
        assert_eq!(a.encrypt(""), "");
    }

    #[test]
    fn key_file_is_reused() {
        let dir = tempfile::tempdir().unwrap();
        let e = Cipher::load(dir.path()).unwrap().encrypt("x");
        assert_eq!(Cipher::load(dir.path()).unwrap().decrypt(&e).unwrap(), "x");
    }
}
