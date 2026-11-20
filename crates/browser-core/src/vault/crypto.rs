use std::fmt;

use argon2::{Algorithm, Argon2, Params, Version};
use chacha20poly1305::aead::{Aead, KeyInit, Payload};
use chacha20poly1305::{XChaCha20Poly1305, XNonce};
use serde::{Deserialize, Serialize};
use thiserror::Error;
use zeroize::{Zeroize, ZeroizeOnDrop, Zeroizing};

pub const KEY_LEN: usize = 32;
pub const NONCE_LEN: usize = 24;
pub const SALT_LEN: usize = 16;

// OWASP-recommended range for interactive unlock; runs once per unlock.
const ARGON2_MEMORY_KIB: u32 = 64 * 1024;
const ARGON2_ITERATIONS: u32 = 3;
const ARGON2_PARALLELISM: u32 = 1;

#[derive(Debug, Error)]
pub enum CryptoError {
    #[error("OS random number generator failed: {0}")]
    Rng(#[from] getrandom::Error),
    #[error("key derivation failed: {0}")]
    Kdf(#[from] argon2::Error),
    /// Wrong key, wrong AAD or tampered data. Deliberately indistinguishable.
    #[error("decryption failed")]
    Decrypt,
    #[error("encryption failed")]
    Encrypt,
}

#[derive(Zeroize, ZeroizeOnDrop)]
pub struct VaultKey([u8; KEY_LEN]);

impl VaultKey {
    pub fn generate() -> Result<Self, CryptoError> {
        let mut key = Self([0; KEY_LEN]);
        getrandom::fill(&mut key.0)?;
        Ok(key)
    }

    pub fn from_bytes(bytes: &[u8]) -> Option<Self> {
        let mut key = Self([0; KEY_LEN]);
        if bytes.len() != KEY_LEN {
            return None;
        }
        key.0.copy_from_slice(bytes);
        Some(key)
    }

    pub fn as_bytes(&self) -> &[u8] {
        &self.0
    }

    fn cipher(&self) -> XChaCha20Poly1305 {
        XChaCha20Poly1305::new(&self.0.into())
    }
}

impl fmt::Debug for VaultKey {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.write_str("VaultKey(..)")
    }
}

/// Nonce plus ciphertext-with-tag. Safe to store and log.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct Sealed {
    pub nonce: [u8; NONCE_LEN],
    pub ciphertext: Vec<u8>,
}

/// Encrypts with a fresh random nonce. 24-byte nonces make random choice safe
/// for any realistic number of records under one key.
pub fn seal(key: &VaultKey, plaintext: &[u8], aad: &[u8]) -> Result<Sealed, CryptoError> {
    let nonce: [u8; NONCE_LEN] = random_bytes()?;
    let payload = Payload {
        msg: plaintext,
        aad,
    };
    let ciphertext = key
        .cipher()
        .encrypt(&XNonce::from(nonce), payload)
        .map_err(|_| CryptoError::Encrypt)?;
    Ok(Sealed { nonce, ciphertext })
}

pub fn open(
    key: &VaultKey,
    sealed: &Sealed,
    aad: &[u8],
) -> Result<Zeroizing<Vec<u8>>, CryptoError> {
    let payload = Payload {
        msg: sealed.ciphertext.as_slice(),
        aad,
    };
    key.cipher()
        .decrypt(&XNonce::from(sealed.nonce), payload)
        .map(Zeroizing::new)
        .map_err(|_| CryptoError::Decrypt)
}

pub fn random_bytes<const N: usize>() -> Result<[u8; N], CryptoError> {
    let mut bytes = [0u8; N];
    getrandom::fill(&mut bytes)?;
    Ok(bytes)
}

/// Argon2id settings, stored in the vault header so they can be raised later
/// without breaking existing vaults.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct KdfParams {
    pub salt: [u8; SALT_LEN],
    pub memory_kib: u32,
    pub iterations: u32,
    pub parallelism: u32,
}

impl KdfParams {
    pub fn generate() -> Result<Self, CryptoError> {
        Ok(Self {
            salt: random_bytes()?,
            memory_kib: ARGON2_MEMORY_KIB,
            iterations: ARGON2_ITERATIONS,
            parallelism: ARGON2_PARALLELISM,
        })
    }

    pub fn derive(&self, password: &[u8]) -> Result<VaultKey, CryptoError> {
        let params = Params::new(
            self.memory_kib,
            self.iterations,
            self.parallelism,
            Some(KEY_LEN),
        )?;
        let argon = Argon2::new(Algorithm::Argon2id, Version::V0x13, params);
        let mut key = VaultKey([0; KEY_LEN]);
        argon.hash_password_into(password, &self.salt, &mut key.0)?;
        Ok(key)
    }

    #[cfg(test)]
    pub fn fast_for_tests() -> Result<Self, CryptoError> {
        Ok(Self {
            memory_kib: 64,
            iterations: 1,
            ..Self::generate()?
        })
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn seal_open_round_trip() -> Result<(), CryptoError> {
        let key = VaultKey::generate()?;
        let sealed = seal(&key, b"hunter2", b"aad")?;
        assert_eq!(open(&key, &sealed, b"aad")?.as_slice(), b"hunter2");
        Ok(())
    }

    #[test]
    fn nonces_differ_per_seal() -> Result<(), CryptoError> {
        let key = VaultKey::generate()?;
        let a = seal(&key, b"same", b"")?;
        let b = seal(&key, b"same", b"")?;
        assert_ne!(a.nonce, b.nonce);
        assert_ne!(a.ciphertext, b.ciphertext);
        Ok(())
    }

    #[test]
    fn wrong_aad_key_or_tamper_fails() -> Result<(), CryptoError> {
        let key = VaultKey::generate()?;
        let mut sealed = seal(&key, b"secret", b"origin-a")?;
        assert!(open(&key, &sealed, b"origin-b").is_err());
        assert!(open(&VaultKey::generate()?, &sealed, b"origin-a").is_err());
        sealed.ciphertext[0] ^= 1;
        assert!(open(&key, &sealed, b"origin-a").is_err());
        Ok(())
    }

    #[test]
    fn kdf_is_deterministic_per_salt() -> Result<(), CryptoError> {
        let params = KdfParams::fast_for_tests()?;
        let a = params.derive(b"pw")?;
        assert_eq!(a.as_bytes(), params.derive(b"pw")?.as_bytes());
        assert_ne!(a.as_bytes(), params.derive(b"pW")?.as_bytes());
        Ok(())
    }
}
