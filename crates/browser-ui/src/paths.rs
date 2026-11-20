use std::path::PathBuf;

use browser_core::WorkspaceId;

/// Where Umbrella keeps its data. `UMBRELLA_HOME` overrides the default
/// `%LOCALAPPDATA%\Umbrella`, which keeps test runs away from real data.
#[derive(Debug, Clone)]
pub struct Paths {
    root: PathBuf,
}

impl Paths {
    pub fn resolve() -> Self {
        let root = std::env::var_os("UMBRELLA_HOME")
            .map(PathBuf::from)
            .or_else(|| std::env::var_os("LOCALAPPDATA").map(|d| PathBuf::from(d).join("Umbrella")))
            .unwrap_or_else(|| std::env::temp_dir().join("Umbrella"));
        Self { root }
    }

    pub fn root(&self) -> &std::path::Path {
        &self.root
    }

    pub fn ensure(&self) -> std::io::Result<()> {
        for dir in [
            self.root.clone(),
            self.themes(),
            self.filters(),
            self.scripts(),
        ] {
            std::fs::create_dir_all(dir)?;
        }
        Ok(())
    }

    pub fn profile_db(&self) -> PathBuf {
        self.root.join("profile.redb")
    }

    pub fn vault(&self) -> PathBuf {
        self.root.join("vault.redb")
    }

    pub fn themes(&self) -> PathBuf {
        self.root.join("themes")
    }

    /// The compiled filter engine, rebuilt whenever the lists change.
    pub fn adblock_cache(&self) -> PathBuf {
        self.root.join("adblock.bin")
    }

    pub fn favicons(&self) -> PathBuf {
        self.root.join("favicons")
    }

    pub fn filters(&self) -> PathBuf {
        self.root.join("filters")
    }

    /// Extensions installed from .zip files are unpacked here.
    pub fn extensions(&self) -> PathBuf {
        self.root.join("extensions")
    }

    pub fn scripts(&self) -> PathBuf {
        self.root.join("scripts")
    }

    /// One WebView2 data folder per workspace keeps cookies and storage apart.
    pub fn webview_data(&self, workspace: WorkspaceId) -> PathBuf {
        self.root
            .join("webview")
            .join(format!("workspace-{}", workspace.0))
    }
}
