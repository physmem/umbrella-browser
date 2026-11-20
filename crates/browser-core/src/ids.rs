use std::fmt;

use serde::{Deserialize, Serialize};

#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, PartialOrd, Ord, Serialize, Deserialize)]
#[serde(transparent)]
pub struct TabId(pub u64);

#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, PartialOrd, Ord, Serialize, Deserialize)]
#[serde(transparent)]
pub struct WorkspaceId(pub u64);

#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, PartialOrd, Ord, Serialize, Deserialize)]
#[serde(transparent)]
pub struct CredentialId(pub u64);

#[derive(Debug, Clone, PartialEq, Eq, Hash, PartialOrd, Ord, Serialize, Deserialize)]
#[serde(transparent)]
pub struct ThemeId(pub String);

impl ThemeId {
    pub fn new(id: impl Into<String>) -> Self {
        Self(id.into())
    }

    pub fn as_str(&self) -> &str {
        &self.0
    }
}

impl fmt::Display for ThemeId {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.write_str(&self.0)
    }
}

impl CredentialId {
    /// Random so IDs never collide across devices or recreated vaults.
    pub fn random() -> Result<Self, getrandom::Error> {
        let mut bytes = [0u8; 8];
        getrandom::fill(&mut bytes)?;
        Ok(Self(u64::from_le_bytes(bytes)))
    }

    pub fn to_key(self) -> String {
        format!("{:016x}", self.0)
    }
}

/// Monotonic allocator for tab and workspace IDs. Persisted with the session
/// so restored tabs never collide with new ones.
#[derive(Debug, Clone, Default, PartialEq, Eq, Serialize, Deserialize)]
pub struct IdAllocator {
    next: u64,
}

impl IdAllocator {
    pub fn tab(&mut self) -> TabId {
        TabId(self.bump())
    }

    pub fn workspace(&mut self) -> WorkspaceId {
        WorkspaceId(self.bump())
    }

    fn bump(&mut self) -> u64 {
        self.next += 1;
        self.next
    }
}
