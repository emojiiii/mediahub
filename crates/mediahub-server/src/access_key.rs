use std::collections::BTreeMap;

use aes_gcm::{
    Aes256Gcm, Nonce,
    aead::{Aead, Generate, Key, KeyInit},
};
use base64::{Engine, engine::general_purpose::URL_SAFE_NO_PAD};
use thiserror::Error;

const NONCE_BYTES: usize = 12;
const KEY_BYTES: usize = 32;

#[derive(Clone)]
pub struct AccessKeyCipher {
    ciphers: BTreeMap<u32, Aes256Gcm>,
    active_version: u32,
}

#[derive(Debug, Error, PartialEq, Eq)]
pub enum AccessKeyCipherError {
    #[error("master key must be a base64-encoded 32-byte value")]
    InvalidMasterKey,
    #[error("encrypted access key secret is invalid")]
    InvalidCiphertext,
    #[error("access key secret encryption failed")]
    EncryptionFailed,
    #[error("access key secret decryption failed")]
    DecryptionFailed,
    #[error("access key was encrypted with an unavailable master key version")]
    UnknownKeyVersion,
}

impl AccessKeyCipher {
    pub fn from_base64(value: &str, version: u32) -> Result<Self, AccessKeyCipherError> {
        Self::from_keyring(version, [(version, value)])
    }

    pub fn from_keyring<'a>(
        active_version: u32,
        values: impl IntoIterator<Item = (u32, &'a str)>,
    ) -> Result<Self, AccessKeyCipherError> {
        if active_version == 0 {
            return Err(AccessKeyCipherError::InvalidMasterKey);
        }
        let mut ciphers = BTreeMap::new();
        for (version, value) in values {
            if version == 0 || ciphers.contains_key(&version) {
                return Err(AccessKeyCipherError::InvalidMasterKey);
            }
            let key = URL_SAFE_NO_PAD
                .decode(value)
                .or_else(|_| base64::engine::general_purpose::STANDARD.decode(value))
                .map_err(|_| AccessKeyCipherError::InvalidMasterKey)?;
            if key.len() != KEY_BYTES {
                return Err(AccessKeyCipherError::InvalidMasterKey);
            }
            let cipher = Aes256Gcm::new_from_slice(&key)
                .map_err(|_| AccessKeyCipherError::InvalidMasterKey)?;
            ciphers.insert(version, cipher);
        }
        if !ciphers.contains_key(&active_version) {
            return Err(AccessKeyCipherError::InvalidMasterKey);
        }
        Ok(Self {
            ciphers,
            active_version,
        })
    }

    #[must_use]
    pub const fn version(&self) -> u32 {
        self.active_version
    }

    #[must_use]
    pub fn supports_version(&self, version: u32) -> bool {
        self.ciphers.contains_key(&version)
    }

    pub fn encrypt(&self, secret: &[u8]) -> Result<String, AccessKeyCipherError> {
        let nonce = Nonce::generate();
        let ciphertext = self
            .ciphers
            .get(&self.active_version)
            .expect("active cipher is validated during construction")
            .encrypt(&nonce, secret)
            .map_err(|_| AccessKeyCipherError::EncryptionFailed)?;
        let mut encoded = Vec::with_capacity(NONCE_BYTES + ciphertext.len());
        encoded.extend_from_slice(&nonce);
        encoded.extend_from_slice(&ciphertext);
        Ok(URL_SAFE_NO_PAD.encode(encoded))
    }

    pub fn decrypt(
        &self,
        ciphertext: &str,
        key_version: u32,
    ) -> Result<Vec<u8>, AccessKeyCipherError> {
        let cipher = self
            .ciphers
            .get(&key_version)
            .ok_or(AccessKeyCipherError::UnknownKeyVersion)?;
        let encoded = URL_SAFE_NO_PAD
            .decode(ciphertext)
            .map_err(|_| AccessKeyCipherError::InvalidCiphertext)?;
        let (nonce, ciphertext) = encoded
            .split_at_checked(NONCE_BYTES)
            .ok_or(AccessKeyCipherError::InvalidCiphertext)?;
        let nonce = Nonce::try_from(nonce).map_err(|_| AccessKeyCipherError::InvalidCiphertext)?;
        cipher
            .decrypt(&nonce, ciphertext)
            .map_err(|_| AccessKeyCipherError::DecryptionFailed)
    }
}

#[must_use]
pub fn generate_secret() -> String {
    URL_SAFE_NO_PAD.encode(Key::<Aes256Gcm>::generate())
}

#[cfg(test)]
mod tests {
    use super::*;

    const MASTER_KEY: &str = "AAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAA";

    #[test]
    fn keyring_decrypts_previous_versions_but_encrypts_with_the_active_version() {
        let cipher = AccessKeyCipher::from_base64(MASTER_KEY, 7).expect("cipher");
        let encrypted = cipher.encrypt(b"secret").expect("encrypt");
        assert_eq!(cipher.decrypt(&encrypted, 7), Ok(b"secret".to_vec()));
        assert!(cipher.supports_version(7));
        assert!(!cipher.supports_version(8));
        let keyring = AccessKeyCipher::from_keyring(
            8,
            [
                (7, MASTER_KEY),
                (8, "AQEBAQEBAQEBAQEBAQEBAQEBAQEBAQEBAQEBAQEBAQE"),
            ],
        )
        .expect("keyring");
        assert_eq!(keyring.decrypt(&encrypted, 7), Ok(b"secret".to_vec()));
        assert_eq!(
            cipher.decrypt(&encrypted, 8),
            Err(AccessKeyCipherError::UnknownKeyVersion)
        );
        let new_encrypted = keyring.encrypt(b"new secret").expect("encrypt");
        assert_eq!(
            keyring.decrypt(&new_encrypted, 8),
            Ok(b"new secret".to_vec())
        );
    }

    #[test]
    fn generated_secret_is_url_safe_and_not_empty() {
        let secret = generate_secret();
        assert_eq!(secret.len(), 43);
        assert!(
            secret
                .bytes()
                .all(|byte| byte.is_ascii_alphanumeric() || byte == b'-' || byte == b'_')
        );
    }
}
