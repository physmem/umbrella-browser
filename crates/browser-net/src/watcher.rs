use std::path::Path;

use notify::{EventKind, RecommendedWatcher, RecursiveMode, Watcher};

/// Calls `on_change` from a background thread whenever files in the watched
/// folders change. Bursts are not coalesced here; pair it with the core
/// `Debouncer` on the UI side. Dropping the watcher stops it.
pub struct FolderWatcher {
    _inner: RecommendedWatcher,
}

impl FolderWatcher {
    /// Folders that do not exist yet are skipped rather than failing startup.
    pub fn watch<F>(dirs: &[&Path], on_change: F) -> notify::Result<Self>
    where
        F: Fn() + Send + 'static,
    {
        let mut inner =
            notify::recommended_watcher(move |event: notify::Result<notify::Event>| {
                if event.is_ok_and(|e| is_content_change(&e.kind)) {
                    on_change();
                }
            })?;
        for dir in dirs.iter().filter(|d| d.is_dir()) {
            inner.watch(dir, RecursiveMode::NonRecursive)?;
        }
        Ok(Self { _inner: inner })
    }
}

fn is_content_change(kind: &EventKind) -> bool {
    matches!(
        kind,
        EventKind::Create(_) | EventKind::Modify(_) | EventKind::Remove(_)
    )
}

#[cfg(test)]
mod tests {
    use std::sync::mpsc;
    use std::time::Duration;

    use super::*;

    #[test]
    fn reports_new_files() -> Result<(), Box<dyn std::error::Error>> {
        let dir = tempfile::tempdir()?;
        let (tx, rx) = mpsc::channel();
        let _watcher = FolderWatcher::watch(&[dir.path()], move || {
            let _ = tx.send(());
        })?;
        std::fs::write(dir.path().join("new.user.js"), "x")?;
        rx.recv_timeout(Duration::from_secs(5))?;
        Ok(())
    }
}
