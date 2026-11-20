//! Core browser state with no UI or engine dependencies: workspaces and tab
//! trees, MRU order, session persistence, theme tokens, spring motion and the
//! encrypted password vault.

pub mod debounce;
pub mod fuzzy;
pub mod history;
pub mod ids;
pub mod import;
pub mod mru;
pub mod permissions;
pub mod session;
pub mod settings;
pub mod spring;
pub mod store;
pub mod tab;
pub mod theme;
pub mod vault;
pub mod workspace;

pub use ids::{CredentialId, TabId, ThemeId, WorkspaceId};
pub use session::Session;
pub use tab::SavedTab;
pub use workspace::Workspace;
