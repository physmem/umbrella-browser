use std::path::Path;

use serde::{Deserialize, Serialize};
use thiserror::Error;

use crate::ids::IdAllocator;
use crate::store::{KvStore, StoreError};
use crate::{SavedTab, TabId, Workspace, WorkspaceId};

pub const SESSION_VERSION: u32 = 1;

const SESSION_TABLE: &str = "session";
const CURRENT_KEY: &str = "current";

#[derive(Debug, Error)]
pub enum SessionError {
    #[error("workspace {0:?} does not exist")]
    UnknownWorkspace(WorkspaceId),
    #[error("session file is version {0}, this build reads up to {SESSION_VERSION}")]
    UnsupportedVersion(u32),
    #[error("session data is corrupt: {0}")]
    Corrupt(#[from] serde_json::Error),
    #[error(transparent)]
    Store(#[from] StoreError),
}

/// Everything needed to rebuild the window on the next launch.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct Session {
    pub version: u32,
    pub workspaces: Vec<Workspace>,
    pub active_workspace: WorkspaceId,
    ids: IdAllocator,
}

impl Session {
    pub fn new() -> Self {
        let mut ids = IdAllocator::default();
        let main = Workspace::new(ids.workspace(), "Main");
        Self {
            version: SESSION_VERSION,
            active_workspace: main.id,
            workspaces: vec![main],
            ids,
        }
    }

    pub fn workspace(&self, id: WorkspaceId) -> Option<&Workspace> {
        self.workspaces.iter().find(|w| w.id == id)
    }

    pub fn workspace_mut(&mut self, id: WorkspaceId) -> Result<&mut Workspace, SessionError> {
        self.workspaces
            .iter_mut()
            .find(|w| w.id == id)
            .ok_or(SessionError::UnknownWorkspace(id))
    }

    pub fn active(&self) -> Option<&Workspace> {
        self.workspace(self.active_workspace)
    }

    pub fn add_workspace(&mut self, name: impl Into<String>) -> WorkspaceId {
        let workspace = Workspace::new(self.ids.workspace(), name);
        let id = workspace.id;
        self.workspaces.push(workspace);
        id
    }

    pub fn switch_workspace(&mut self, id: WorkspaceId) -> Result<(), SessionError> {
        self.workspace_mut(id)?;
        self.active_workspace = id;
        Ok(())
    }

    pub fn open_tab(
        &mut self,
        workspace: WorkspaceId,
        url: impl Into<String>,
        parent: Option<TabId>,
    ) -> Result<TabId, SessionError> {
        let id = self.ids.tab();
        let ws = self.workspace_mut(workspace)?;
        ws.insert_tab(SavedTab::new(id, url, parent));
        ws.activate(id);
        Ok(id)
    }

    pub fn to_json(&self) -> Result<Vec<u8>, SessionError> {
        Ok(serde_json::to_vec(self)?)
    }

    pub fn from_json(bytes: &[u8]) -> Result<Self, SessionError> {
        let session: Session = serde_json::from_slice(bytes)?;
        if session.version > SESSION_VERSION {
            return Err(SessionError::UnsupportedVersion(session.version));
        }
        Ok(session)
    }
}

impl Default for Session {
    fn default() -> Self {
        Self::new()
    }
}

/// Session persistence on its own redb file.
pub struct SessionStore {
    kv: KvStore,
}

impl SessionStore {
    pub fn open(path: &Path) -> Result<Self, SessionError> {
        Ok(Self::with_store(KvStore::open(path)?))
    }

    pub fn with_store(kv: KvStore) -> Self {
        Self { kv }
    }

    pub fn save(&self, session: &Session) -> Result<(), SessionError> {
        self.kv
            .put(SESSION_TABLE, CURRENT_KEY, &session.to_json()?)?;
        Ok(())
    }

    /// `None` on first launch. A corrupt or newer file is an error, never a
    /// silent reset, so the caller can back it up before starting fresh.
    pub fn load(&self) -> Result<Option<Session>, SessionError> {
        let Some(bytes) = self.kv.get(SESSION_TABLE, CURRENT_KEY)? else {
            return Ok(None);
        };
        Session::from_json(&bytes).map(Some)
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn round_trips_through_store() -> Result<(), Box<dyn std::error::Error>> {
        let dir = tempfile::tempdir()?;
        let store = SessionStore::open(&dir.path().join("session.redb"))?;
        assert!(store.load()?.is_none());

        let mut session = Session::new();
        let ws = session.active_workspace;
        let parent = session.open_tab(ws, "https://a.test/", None)?;
        session.open_tab(ws, "https://b.test/", Some(parent))?;
        store.save(&session)?;

        assert_eq!(store.load()?, Some(session));
        Ok(())
    }

    #[test]
    fn ids_stay_unique_after_restore() -> Result<(), SessionError> {
        let mut session = Session::new();
        let ws = session.active_workspace;
        let first = session.open_tab(ws, "https://a.test/", None)?;
        let mut restored = Session::from_json(&session.to_json()?)?;
        let second = restored.open_tab(ws, "https://b.test/", None)?;
        assert_ne!(first, second);
        Ok(())
    }

    #[test]
    fn rejects_newer_version() -> Result<(), SessionError> {
        let mut session = Session::new();
        session.version = SESSION_VERSION + 1;
        let result = Session::from_json(&session.to_json()?);
        assert!(matches!(result, Err(SessionError::UnsupportedVersion(_))));
        Ok(())
    }
}
