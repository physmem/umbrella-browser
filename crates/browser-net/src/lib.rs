//! Network-side policy with no engine dependency: ad and tracker blocking,
//! tracking-parameter stripping, and userscript / user CSS compilation.

pub mod blocker;
pub mod clean;
pub mod exposure;
pub mod favicons;
pub mod lists;
pub mod userscript;
pub mod watcher;

pub use blocker::{Blocker, ResourceKind};
pub use clean::strip_tracking;
pub use exposure::Exposure;
pub use favicons::FaviconCache;
pub use userscript::{ScriptLibrary, UserScript};
pub use watcher::FolderWatcher;
