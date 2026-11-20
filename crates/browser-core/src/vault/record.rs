use std::fmt;

use serde::{Deserialize, Serialize};
use zeroize::Zeroizing;

use super::crypto::Sealed;
use super::origin::Origin;
use crate::CredentialId;

const AAD_DOMAIN: &str = "umbrella/credential/v1";
const LEN_PREFIX: usize = 4;

/// A decrypted login. Both fields wipe themselves on drop.
pub struct Credential {
    pub username: Zeroizing<String>,
    pub password: Zeroizing<String>,
}

impl Credential {
    pub fn new(username: &str, password: &str) -> Self {
        Self {
            username: Zeroizing::new(username.to_owned()),
            password: Zeroizing::new(password.to_owned()),
        }
    }

    /// `[u32 LE username length][username][password]`. The buffer is sized
    /// up front so it never reallocates and strands an unwiped copy.
    pub(crate) fn encode(&self) -> Zeroizing<Vec<u8>> {
        let user = self.username.as_bytes();
        let pass = self.password.as_bytes();
        let mut out = Zeroizing::new(Vec::with_capacity(LEN_PREFIX + user.len() + pass.len()));
        out.extend_from_slice(&(user.len() as u32).to_le_bytes());
        out.extend_from_slice(user);
        out.extend_from_slice(pass);
        out
    }

    pub(crate) fn decode(bytes: &[u8]) -> Option<Self> {
        let (prefix, rest) = bytes.split_at_checked(LEN_PREFIX)?;
        let user_len = u32::from_le_bytes(prefix.try_into().ok()?) as usize;
        let (user, pass) = rest.split_at_checked(user_len)?;
        Some(Self {
            username: Zeroizing::new(std::str::from_utf8(user).ok()?.to_owned()),
            password: Zeroizing::new(std::str::from_utf8(pass).ok()?.to_owned()),
        })
    }
}

impl fmt::Debug for Credential {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.debug_struct("Credential")
            .field("username", &"..")
            .field("password", &"..")
            .finish()
    }
}

/// What lands on disk. The origin stays in clear for lookup; everything
/// else secret is inside `sealed`.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct SealedCredential {
    pub id: CredentialId,
    pub origin: Origin,
    pub sealed: Sealed,
    pub created: u64,
    pub updated: u64,
}

/// Enough to render an account picker. No password.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct CredentialSummary {
    pub id: CredentialId,
    pub origin: Origin,
    pub username: String,
}

/// Binds a ciphertext to one record at one origin: copying it into another
/// record, or pointing the record at another origin, makes it fail to open.
pub(crate) fn aad(origin: &Origin, id: CredentialId) -> Vec<u8> {
    format!("{AAD_DOMAIN}\0{}\0{}", origin.canonical(), id.to_key()).into_bytes()
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn encode_round_trip() -> Result<(), &'static str> {
        let cred = Credential::new("ada@example.com", "pässwörd:with\0bytes");
        let decoded = Credential::decode(&cred.encode()).ok_or("decode failed")?;
        assert_eq!(*decoded.username, *cred.username);
        assert_eq!(*decoded.password, *cred.password);
        Ok(())
    }

    #[test]
    fn decode_rejects_truncated() {
        assert!(Credential::decode(&[1, 0]).is_none());
        assert!(Credential::decode(&[9, 0, 0, 0, b'a']).is_none());
    }

    #[test]
    fn debug_hides_secrets() {
        let text = format!("{:?}", Credential::new("ada", "hunter2"));
        assert!(!text.contains("hunter2") && !text.contains("ada"));
    }
}
