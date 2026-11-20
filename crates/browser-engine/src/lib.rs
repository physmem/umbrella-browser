//! WebView2 integration. Everything here runs on the UI thread, which must be
//! a COM STA with a message loop; see `docs/decisions/0002-webview2-ownership.md`.

pub mod bounds;
pub mod events;
pub mod hibernate;
pub mod login;
pub mod shortcuts;

#[cfg(windows)]
mod cookies;
#[cfg(windows)]
mod downloads;
#[cfg(windows)]
mod error;
#[cfg(windows)]
mod extensions;
#[cfg(windows)]
mod find;
#[cfg(windows)]
mod inspect;
#[cfg(windows)]
mod permissions;
#[cfg(windows)]
mod profile;
#[cfg(windows)]
mod tab;

pub use bounds::{LogicalRect, PhysicalRect};
pub use events::{EngineEvent, EventQueue, TabEvent};
pub use hibernate::{HibernateAction, HibernatePolicy, Residency, TabActivity};
pub use shortcuts::Shortcut;

#[cfg(windows)]
#[cfg(windows)]
pub use cookies::{ImportCookie, SameSite};
#[cfg(windows)]
pub use downloads::{DownloadId, DownloadTarget, unique_path};
#[cfg(windows)]
pub use error::EngineError;
#[cfg(windows)]
pub use extensions::ExtensionInfo;
#[cfg(windows)]
pub use inspect::{DocumentResponse, PAGE_STATS_SCRIPT, RendererMemory};
#[cfg(windows)]
pub use permissions::{PermissionPolicy, origin_of};
#[cfg(windows)]
pub use profile::{Profile, ProfileOptions};
#[cfg(windows)]
pub use tab::{PageSettings, RequestFilter, TabHooks, TabView};
