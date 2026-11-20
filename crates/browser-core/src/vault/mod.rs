//! Encrypted password vault. See `docs/decisions/0001-vault-and-storage.md`
//! for the key lifecycle.

pub mod crypto;
pub mod dpapi;
pub mod origin;
pub mod record;

use std::cell::Cell;
use std::path::Path;
use std::time::{Duration, Instant, SystemTime, UNIX_EPOCH};

use serde::{Deserialize, Serialize};
use thiserror::Error;
use zeroize::Zeroizing;

use crate::CredentialId;
use crate::store::{KvStore, StoreError};
use crypto::{CryptoError, Sealed};
pub use crypto::{KdfParams, VaultKey};
use dpapi::DpapiError;
pub use origin::{Origin, OriginError};
pub use record::{Credential, CredentialSummary, SealedCredential};

pub const VAULT_VERSION: u32 = 1;
pub const DEFAULT_IDLE_LOCK: Duration = Duration::from_secs(15 * 60);

const HEADER_TABLE: &str = "header";
const HEADER_KEY: &str = "current";
const CREDENTIALS_TABLE: &str = "credentials";
const KEY_WRAP_AAD: &[u8] = b"umbrella/vault-key/v1";

#[derive(Debug, Error)]
pub enum VaultError {
    #[error("vault is locked")]
    Locked,
    #[error("wrong master password")]
    WrongPassword,
    #[error("no vault exists at this path")]
    NotFound,
    #[error("a vault already exists at this path")]
    AlreadyExists,
    #[error("master password must not be empty")]
    EmptyPassword,
    #[error("DPAPI unlock is not set up for this vault")]
    DpapiNotEnabled,
    #[error("no credential with that id")]
    UnknownCredential,
    #[error("vault data is corrupt: {0}")]
    Corrupt(&'static str),
    #[error("vault file is version {0}, this build reads up to {VAULT_VERSION}")]
    UnsupportedVersion(u32),
    #[error("vault data is corrupt: {0}")]
    Json(#[from] serde_json::Error),
    #[error(transparent)]
    Crypto(#[from] CryptoError),
    #[error(transparent)]
    Dpapi(#[from] DpapiError),
    #[error(transparent)]
    Store(#[from] StoreError),
}

/// What the save prompt should offer after a login form submits.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum SaveAction {
    Save,
    Update(CredentialId),
    Unchanged,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
struct VaultHeader {
    version: u32,
    kdf: KdfParams,
    password_wrap: Sealed,
    #[serde(default)]
    dpapi_wrap: Option<Vec<u8>>,
}

struct Unlocked {
    key: VaultKey,
    last_used: Cell<Instant>,
}

pub struct Vault {
    kv: KvStore,
    header: VaultHeader,
    unlocked: Option<Unlocked>,
}

impl Vault {
    /// Creates a new vault, unlocked. A password is required: DPAPI blobs do
    /// not survive a Windows reinstall, so the password is the recovery path.
    pub fn create(path: &Path, password: &str) -> Result<Self, VaultError> {
        Self::create_with_kdf(path, password, KdfParams::generate()?)
    }

    fn create_with_kdf(path: &Path, password: &str, kdf: KdfParams) -> Result<Self, VaultError> {
        if password.is_empty() {
            return Err(VaultError::EmptyPassword);
        }
        let kek = kdf.derive(password.as_bytes())?;
        Self::create_with_kek(path, kdf, &kek)
    }

    /// Creates a vault from a key-encryption key derived elsewhere. Argon2id
    /// is the slow part of creating or unlocking, so callers with a UI run
    /// `KdfParams::derive` on a worker thread and finish here.
    pub fn create_with_kek(
        path: &Path,
        kdf: KdfParams,
        kek: &VaultKey,
    ) -> Result<Self, VaultError> {
        let kv = KvStore::open(path)?;
        if read_header(&kv)?.is_some() {
            return Err(VaultError::AlreadyExists);
        }
        let key = VaultKey::generate()?;
        let header = VaultHeader {
            version: VAULT_VERSION,
            password_wrap: crypto::seal(kek, key.as_bytes(), KEY_WRAP_AAD)?,
            kdf,
            dpapi_wrap: None,
        };
        write_header(&kv, &header)?;
        Ok(Self {
            kv,
            header,
            unlocked: Some(Unlocked::new(key)),
        })
    }

    /// Opens an existing vault, locked.
    pub fn open(path: &Path) -> Result<Self, VaultError> {
        let kv = KvStore::open(path)?;
        let header = read_header(&kv)?.ok_or(VaultError::NotFound)?;
        if header.version > VAULT_VERSION {
            return Err(VaultError::UnsupportedVersion(header.version));
        }
        Ok(Self {
            kv,
            header,
            unlocked: None,
        })
    }

    pub fn is_locked(&self) -> bool {
        self.unlocked.is_none()
    }

    pub fn has_dpapi(&self) -> bool {
        self.header.dpapi_wrap.is_some()
    }

    pub fn unlock(&mut self, password: &str) -> Result<(), VaultError> {
        let kek = self.header.kdf.derive(password.as_bytes())?;
        self.unlock_with_kek(&kek)
    }

    /// The KDF settings to derive a key-encryption key with, off-thread.
    pub fn kdf_params(&self) -> KdfParams {
        self.header.kdf.clone()
    }

    /// Finishes an unlock whose Argon2id step ran elsewhere.
    pub fn unlock_with_kek(&mut self, kek: &VaultKey) -> Result<(), VaultError> {
        let raw =
            crypto::open(kek, &self.header.password_wrap, KEY_WRAP_AAD).map_err(|e| match e {
                CryptoError::Decrypt => VaultError::WrongPassword,
                other => other.into(),
            })?;
        self.set_unlocked(&raw)
    }

    pub fn unlock_with_dpapi(&mut self) -> Result<(), VaultError> {
        let blob = self
            .header
            .dpapi_wrap
            .as_ref()
            .ok_or(VaultError::DpapiNotEnabled)?;
        let raw = dpapi::unprotect(blob, KEY_WRAP_AAD)?;
        self.set_unlocked(&raw)
    }

    /// Drops the key; `VaultKey` wipes itself.
    pub fn lock(&mut self) {
        self.unlocked = None;
    }

    /// Call from the UI tick. Returns true if this call locked the vault.
    pub fn lock_if_idle(&mut self, now: Instant, timeout: Duration) -> bool {
        let idle = self
            .unlocked
            .as_ref()
            .is_some_and(|u| now.saturating_duration_since(u.last_used.get()) >= timeout);
        if idle {
            self.lock();
        }
        idle
    }

    pub fn enable_dpapi(&mut self) -> Result<(), VaultError> {
        let blob = dpapi::protect(self.key()?.as_bytes(), KEY_WRAP_AAD)?;
        self.header.dpapi_wrap = Some(blob);
        write_header(&self.kv, &self.header)
    }

    pub fn disable_dpapi(&mut self) -> Result<(), VaultError> {
        self.header.dpapi_wrap = None;
        write_header(&self.kv, &self.header)
    }

    /// Rewraps the same vault key under a new password and fresh salt;
    /// records are untouched.
    pub fn change_password(&mut self, new_password: &str) -> Result<(), VaultError> {
        let kdf = KdfParams::generate()?;
        let wrap = wrap_with_password(self.key()?, &kdf, new_password)?;
        self.header.kdf = kdf;
        self.header.password_wrap = wrap;
        write_header(&self.kv, &self.header)
    }

    /// Accounts to offer in the fill prompt for this exact origin.
    pub fn accounts_for(&self, origin: &Origin) -> Result<Vec<CredentialSummary>, VaultError> {
        let mut out = Vec::new();
        for record in self.records_for(origin)? {
            let cred = self.decrypt(&record)?;
            out.push(CredentialSummary {
                id: record.id,
                origin: record.origin,
                username: cred.username.to_string(),
            });
        }
        Ok(out)
    }

    /// Decrypts one password for an immediate fill. Requires the same origin
    /// the record was saved for, so a mismatched page cannot pull it.
    pub fn reveal_password(
        &self,
        origin: &Origin,
        id: CredentialId,
    ) -> Result<Zeroizing<String>, VaultError> {
        let record = self.record(id)?.filter(|r| r.origin == *origin);
        let record = record.ok_or(VaultError::UnknownCredential)?;
        let Credential { password, .. } = self.decrypt(&record)?;
        Ok(password)
    }

    pub fn plan_save(&self, origin: &Origin, cred: &Credential) -> Result<SaveAction, VaultError> {
        Ok(match self.find_by_username(origin, &cred.username)? {
            None => SaveAction::Save,
            Some((_, existing)) if *existing.password == *cred.password => SaveAction::Unchanged,
            Some((record, _)) => SaveAction::Update(record.id),
        })
    }

    /// Saves a new login, or replaces the password of the same username at
    /// the same origin.
    pub fn save(&self, origin: &Origin, cred: &Credential) -> Result<CredentialId, VaultError> {
        let existing = self
            .find_by_username(origin, &cred.username)?
            .map(|(r, _)| r);
        let id = match &existing {
            Some(record) => record.id,
            None => CredentialId::random().map_err(CryptoError::from)?,
        };
        let now = unix_now();
        let record = SealedCredential {
            id,
            origin: origin.clone(),
            sealed: crypto::seal(self.key()?, &cred.encode(), &record::aad(origin, id))?,
            created: existing.map_or(now, |r| r.created),
            updated: now,
        };
        self.kv.put(
            CREDENTIALS_TABLE,
            &id.to_key(),
            &serde_json::to_vec(&record)?,
        )?;
        Ok(id)
    }

    pub fn remove(&self, id: CredentialId) -> Result<bool, VaultError> {
        self.key()?;
        Ok(self.kv.delete(CREDENTIALS_TABLE, &id.to_key())?)
    }

    fn key(&self) -> Result<&VaultKey, VaultError> {
        let unlocked = self.unlocked.as_ref().ok_or(VaultError::Locked)?;
        unlocked.last_used.set(Instant::now());
        Ok(&unlocked.key)
    }

    fn set_unlocked(&mut self, raw: &[u8]) -> Result<(), VaultError> {
        let key =
            VaultKey::from_bytes(raw).ok_or(VaultError::Corrupt("vault key has wrong length"))?;
        self.unlocked = Some(Unlocked::new(key));
        Ok(())
    }

    fn decrypt(&self, record: &SealedCredential) -> Result<Credential, VaultError> {
        let aad = record::aad(&record.origin, record.id);
        let plain = crypto::open(self.key()?, &record.sealed, &aad)?;
        Credential::decode(&plain).ok_or(VaultError::Corrupt("credential payload"))
    }

    fn record(&self, id: CredentialId) -> Result<Option<SealedCredential>, VaultError> {
        let Some(bytes) = self.kv.get(CREDENTIALS_TABLE, &id.to_key())? else {
            return Ok(None);
        };
        Ok(Some(serde_json::from_slice(&bytes)?))
    }

    /// Origins are stored in clear (decision 0001), so this works while
    /// locked: the fill prompt can offer "unlock" only where it would help.
    pub fn has_accounts_for(&self, origin: &Origin) -> Result<bool, VaultError> {
        Ok(!self.sealed_records_for(origin)?.is_empty())
    }

    fn records_for(&self, origin: &Origin) -> Result<Vec<SealedCredential>, VaultError> {
        self.key()?;
        self.sealed_records_for(origin)
    }

    fn sealed_records_for(&self, origin: &Origin) -> Result<Vec<SealedCredential>, VaultError> {
        let mut out = Vec::new();
        for bytes in self.kv.values(CREDENTIALS_TABLE)? {
            let record: SealedCredential = serde_json::from_slice(&bytes)?;
            if record.origin == *origin {
                out.push(record);
            }
        }
        Ok(out)
    }

    fn find_by_username(
        &self,
        origin: &Origin,
        username: &str,
    ) -> Result<Option<(SealedCredential, Credential)>, VaultError> {
        for record in self.records_for(origin)? {
            let cred = self.decrypt(&record)?;
            if *cred.username == username {
                return Ok(Some((record, cred)));
            }
        }
        Ok(None)
    }
}

impl Unlocked {
    fn new(key: VaultKey) -> Self {
        Self {
            key,
            last_used: Cell::new(Instant::now()),
        }
    }
}

fn wrap_with_password(
    key: &VaultKey,
    kdf: &KdfParams,
    password: &str,
) -> Result<Sealed, VaultError> {
    if password.is_empty() {
        return Err(VaultError::EmptyPassword);
    }
    let kek = kdf.derive(password.as_bytes())?;
    Ok(crypto::seal(&kek, key.as_bytes(), KEY_WRAP_AAD)?)
}

fn read_header(kv: &KvStore) -> Result<Option<VaultHeader>, VaultError> {
    let Some(bytes) = kv.get(HEADER_TABLE, HEADER_KEY)? else {
        return Ok(None);
    };
    Ok(Some(serde_json::from_slice(&bytes)?))
}

fn write_header(kv: &KvStore, header: &VaultHeader) -> Result<(), VaultError> {
    kv.put(HEADER_TABLE, HEADER_KEY, &serde_json::to_vec(header)?)?;
    Ok(())
}

fn unix_now() -> u64 {
    SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .map_or(0, |d| d.as_secs())
}

#[cfg(test)]
mod tests {
    use super::*;

    type TestResult = Result<(), Box<dyn std::error::Error>>;

    fn new_vault(dir: &Path) -> Result<Vault, VaultError> {
        Vault::create_with_kdf(
            &dir.join("vault.redb"),
            "correct horse",
            KdfParams::fast_for_tests()?,
        )
    }

    fn site() -> Result<Origin, OriginError> {
        Origin::parse("https://example.com/login")
    }

    #[test]
    fn save_lock_unlock_fill() -> TestResult {
        let dir = tempfile::tempdir()?;
        let vault = new_vault(dir.path())?;
        let id = vault.save(&site()?, &Credential::new("ada", "hunter2"))?;
        drop(vault);

        let mut vault = Vault::open(&dir.path().join("vault.redb"))?;
        assert!(vault.is_locked());
        assert!(vault.has_accounts_for(&site()?)?);
        assert!(!vault.has_accounts_for(&Origin::parse("https://other.test/")?)?);
        assert!(matches!(
            vault.accounts_for(&site()?),
            Err(VaultError::Locked)
        ));
        assert!(matches!(
            vault.unlock("wrong"),
            Err(VaultError::WrongPassword)
        ));
        vault.unlock("correct horse")?;

        let accounts = vault.accounts_for(&site()?)?;
        assert_eq!(accounts.len(), 1);
        assert_eq!(accounts[0].username, "ada");
        assert_eq!(vault.reveal_password(&site()?, id)?.as_str(), "hunter2");
        Ok(())
    }

    #[test]
    fn other_origins_see_nothing() -> TestResult {
        let dir = tempfile::tempdir()?;
        let vault = new_vault(dir.path())?;
        let id = vault.save(&site()?, &Credential::new("ada", "hunter2"))?;

        for other in [
            "http://example.com/",
            "https://evil.example.com/",
            "https://example.com:8443/",
        ] {
            let other = Origin::parse(other)?;
            assert!(vault.accounts_for(&other)?.is_empty());
            assert!(matches!(
                vault.reveal_password(&other, id),
                Err(VaultError::UnknownCredential)
            ));
        }
        Ok(())
    }

    #[test]
    fn plan_save_detects_new_update_unchanged() -> TestResult {
        let dir = tempfile::tempdir()?;
        let vault = new_vault(dir.path())?;
        let origin = site()?;
        let first = Credential::new("ada", "hunter2");
        assert_eq!(vault.plan_save(&origin, &first)?, SaveAction::Save);

        let id = vault.save(&origin, &first)?;
        assert_eq!(vault.plan_save(&origin, &first)?, SaveAction::Unchanged);

        let changed = Credential::new("ada", "hunter3");
        assert_eq!(vault.plan_save(&origin, &changed)?, SaveAction::Update(id));
        assert_eq!(vault.save(&origin, &changed)?, id);
        assert_eq!(vault.reveal_password(&origin, id)?.as_str(), "hunter3");
        Ok(())
    }

    #[test]
    fn moved_ciphertext_fails_to_open() -> TestResult {
        let dir = tempfile::tempdir()?;
        let vault = new_vault(dir.path())?;
        let origin = site()?;
        let id = vault.save(&origin, &Credential::new("ada", "hunter2"))?;

        let mut record = vault.record(id)?.ok_or("record missing")?;
        record.origin = Origin::parse("https://evil.test/")?;
        assert!(matches!(
            vault.decrypt(&record),
            Err(VaultError::Crypto(CryptoError::Decrypt))
        ));
        Ok(())
    }

    #[test]
    fn idle_lock_and_change_password() -> TestResult {
        let dir = tempfile::tempdir()?;
        let mut vault = new_vault(dir.path())?;
        assert!(!vault.lock_if_idle(Instant::now(), Duration::from_secs(60)));
        assert!(vault.lock_if_idle(
            Instant::now() + Duration::from_secs(61),
            Duration::from_secs(60)
        ));
        assert!(vault.is_locked());

        vault.unlock("correct horse")?;
        vault.change_password("battery staple")?;
        vault.lock();
        assert!(matches!(
            vault.unlock("correct horse"),
            Err(VaultError::WrongPassword)
        ));
        vault.unlock("battery staple")?;
        Ok(())
    }

    #[test]
    fn split_unlock_matches_password_unlock() -> TestResult {
        let dir = tempfile::tempdir()?;
        let mut vault = new_vault(dir.path())?;
        vault.lock();
        let kek = vault.kdf_params().derive(b"correct horse")?;
        vault.unlock_with_kek(&kek)?;
        assert!(!vault.is_locked());
        vault.lock();
        let wrong = vault.kdf_params().derive(b"nope")?;
        assert!(matches!(
            vault.unlock_with_kek(&wrong),
            Err(VaultError::WrongPassword)
        ));
        Ok(())
    }

    #[test]
    fn refuses_empty_password_and_double_create() -> TestResult {
        let dir = tempfile::tempdir()?;
        let path = dir.path().join("vault.redb");
        assert!(matches!(
            Vault::create_with_kdf(&path, "", KdfParams::fast_for_tests()?),
            Err(VaultError::EmptyPassword)
        ));
        new_vault(dir.path())?;
        assert!(matches!(
            new_vault(dir.path()),
            Err(VaultError::AlreadyExists)
        ));
        Ok(())
    }

    #[cfg(windows)]
    #[test]
    fn dpapi_unlock() -> TestResult {
        let dir = tempfile::tempdir()?;
        let mut vault = new_vault(dir.path())?;
        vault.enable_dpapi()?;
        vault.lock();
        vault.unlock_with_dpapi()?;
        assert!(!vault.is_locked());
        Ok(())
    }
}
