//! Equivalent of Electron `safeStorage`: AES-256-GCM with a key kept in the OS
//! keyring (Secret Service / Credential Manager / Keychain). Values are hex encoded.

use aes_gcm::aead::{Aead, KeyInit};
use aes_gcm::{Aes256Gcm, Nonce};
use rand::RngCore;

const PREFIX: &[u8] = b"ytmd1";
const KEYRING_SERVICE: &str = "YtmRs Safe Storage";
const KEYRING_USER: &str = "ytmrs";

#[derive(Default)]
pub struct SafeStorage {
    key: Option<[u8; 32]>,
}

impl SafeStorage {
    /// Blocking: may talk to the keyring daemon
    pub fn load() -> Self {
        match load_from_keyring() {
            Ok(key) => Self { key: Some(key) },
            Err(err) => {
                log::warn!("safeStorage: keyring unavailable ({err})");
                #[cfg(target_os = "linux")]
                {
                    // Same spirit as Electron's "basic_text" backend fallback on Linux
                    match load_from_file() {
                        Ok(key) => {
                            log::warn!("safeStorage: using local key file fallback");
                            return Self { key: Some(key) };
                        }
                        Err(err) => log::warn!("safeStorage: file fallback failed ({err})"),
                    }
                }
                Self { key: None }
            }
        }
    }

    #[cfg(test)]
    pub fn with_key(key: [u8; 32]) -> Self {
        Self { key: Some(key) }
    }

    pub fn is_available(&self) -> bool {
        self.key.is_some()
    }

    pub fn encrypt_string(&self, plaintext: &str) -> anyhow::Result<String> {
        let key = self.key.ok_or_else(|| anyhow::anyhow!("safeStorage is unavailable"))?;
        let cipher = Aes256Gcm::new_from_slice(&key)?;
        let mut nonce = [0u8; 12];
        rand::rng().fill_bytes(&mut nonce);
        let ciphertext = cipher
            .encrypt(Nonce::from_slice(&nonce), plaintext.as_bytes())
            .map_err(|_| anyhow::anyhow!("encryption failed"))?;
        let mut out = PREFIX.to_vec();
        out.extend_from_slice(&nonce);
        out.extend_from_slice(&ciphertext);
        Ok(hex::encode(out))
    }

    pub fn decrypt_string(&self, hex_value: &str) -> anyhow::Result<String> {
        let key = self.key.ok_or_else(|| anyhow::anyhow!("safeStorage is unavailable"))?;
        let data = hex::decode(hex_value)?;
        if data.len() < PREFIX.len() + 12 || &data[..PREFIX.len()] != PREFIX {
            anyhow::bail!("Error while decrypting the ciphertext provided to safeStorage.decryptString.");
        }
        let (nonce, ciphertext) = data[PREFIX.len()..].split_at(12);
        let cipher = Aes256Gcm::new_from_slice(&key)?;
        let plain = cipher
            .decrypt(Nonce::from_slice(nonce), ciphertext)
            .map_err(|_| anyhow::anyhow!("Error while decrypting the ciphertext provided to safeStorage.decryptString."))?;
        Ok(String::from_utf8(plain)?)
    }
}

fn load_from_keyring() -> anyhow::Result<[u8; 32]> {
    let entry = keyring::Entry::new(KEYRING_SERVICE, KEYRING_USER)?;
    match entry.get_password() {
        Ok(existing) => decode_key(&existing),
        Err(keyring::Error::NoEntry) => {
            let key = random_key();
            entry.set_password(&hex::encode(key))?;
            // Make sure it actually persisted
            let stored = entry.get_password()?;
            decode_key(&stored)
        }
        Err(err) => Err(err.into()),
    }
}

#[cfg(target_os = "linux")]
fn load_from_file() -> anyhow::Result<[u8; 32]> {
    use std::os::unix::fs::PermissionsExt;
    let path = crate::paths::get().user_data.join(".safe-storage-key");
    if let Ok(existing) = std::fs::read_to_string(&path) {
        return decode_key(existing.trim());
    }
    let key = random_key();
    std::fs::write(&path, hex::encode(key))?;
    std::fs::set_permissions(&path, std::fs::Permissions::from_mode(0o600))?;
    Ok(key)
}

fn random_key() -> [u8; 32] {
    let mut key = [0u8; 32];
    rand::rng().fill_bytes(&mut key);
    key
}

fn decode_key(s: &str) -> anyhow::Result<[u8; 32]> {
    let bytes = hex::decode(s.trim())?;
    bytes.try_into().map_err(|_| anyhow::anyhow!("invalid stored key"))
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn roundtrip() {
        let s = SafeStorage::with_key([7u8; 32]);
        let enc = s.encrypt_string("[{\"id\":\"x\"}]").unwrap();
        assert_ne!(enc, "[{\"id\":\"x\"}]");
        assert_eq!(s.decrypt_string(&enc).unwrap(), "[{\"id\":\"x\"}]");
        assert!(s.decrypt_string("deadbeef").is_err());
        assert!(SafeStorage::default().encrypt_string("x").is_err());
        let other = SafeStorage::with_key([8u8; 32]);
        assert!(other.decrypt_string(&enc).is_err());
    }
}
