//! Download tracking for the toolbar pill and the downloads sheet.

use std::path::PathBuf;

use browser_core::TabId;
use browser_engine::DownloadId;

const KEEP: usize = 50;

/// Types that only open in a viewer. Anything that can run code (programs,
/// scripts, installers, shortcuts, Office macros) is never opened for you.
const SAFE_TO_OPEN: &[&str] = &[
    "pdf", "txt", "md", "csv", "png", "jpg", "jpeg", "gif", "webp", "avif", "bmp", "mp3", "m4a",
    "flac", "wav", "ogg", "opus", "mp4", "m4v", "webm", "mkv", "mov",
];

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum DownloadState {
    Active,
    Done,
    Failed,
}

#[derive(Debug, Clone)]
pub struct Download {
    pub id: DownloadId,
    /// The tab that started it, which owns the operation for cancelling.
    pub tab: TabId,
    pub file: PathBuf,
    pub received: u64,
    pub total: Option<u64>,
    pub state: DownloadState,
}

#[derive(Debug, Default)]
pub struct Downloads {
    items: Vec<Download>,
}

impl Downloads {
    pub fn started(&mut self, tab: TabId, id: DownloadId, file: PathBuf, total: Option<u64>) {
        self.items.insert(
            0,
            Download {
                id,
                tab,
                file,
                received: 0,
                total,
                state: DownloadState::Active,
            },
        );
        self.items.truncate(KEEP);
    }

    pub fn progress(&mut self, id: DownloadId, received: u64, total: Option<u64>) {
        if let Some(d) = self.get_mut(id) {
            d.received = received;
            d.total = total.or(d.total);
        }
    }

    /// Returns the download so the caller can act on a finished file.
    pub fn finished(&mut self, id: DownloadId, ok: bool) -> Option<&Download> {
        let d = self.get_mut(id)?;
        d.state = if ok {
            DownloadState::Done
        } else {
            DownloadState::Failed
        };
        Some(d)
    }

    pub fn items(&self) -> &[Download] {
        &self.items
    }

    /// Forgets finished and failed downloads; files stay on disk.
    pub fn clear_finished(&mut self) {
        self.items.retain(|d| d.state == DownloadState::Active);
    }

    pub fn latest(&self) -> Option<&Download> {
        self.items.first()
    }

    pub fn active_count(&self) -> usize {
        self.items
            .iter()
            .filter(|d| d.state == DownloadState::Active)
            .count()
    }

    fn get_mut(&mut self, id: DownloadId) -> Option<&mut Download> {
        self.items.iter_mut().find(|d| d.id == id)
    }
}

impl Download {
    pub fn file_name(&self) -> String {
        self.file
            .file_name()
            .map(|n| n.to_string_lossy().into_owned())
            .unwrap_or_default()
    }

    /// "42%" with a known size, "3.1 MB" without.
    pub fn progress_label(&self) -> String {
        match self.total {
            Some(total) if total > 0 => format!("{}%", (self.received * 100 / total).min(100)),
            _ => format!("{:.1} MB", self.received as f64 / 1_048_576.0),
        }
    }

    /// Opens Explorer with the file selected.
    pub fn reveal(&self) {
        let target = if self.file.exists() {
            self.file.clone()
        } else {
            self.folder()
        };
        let _ = std::process::Command::new("explorer")
            .arg(format!("/select,{}", target.display()))
            .spawn();
    }

    /// True when the type is on the viewer-only list and the file exists.
    pub fn safe_to_open(&self) -> bool {
        is_safe_to_open(&self.file) && self.file.exists()
    }

    /// "Done · 3.1 MB", "Failed" or progress while running.
    pub fn status_label(&self) -> String {
        match self.state {
            DownloadState::Active => self.progress_label(),
            DownloadState::Done => {
                format!("Done · {}", megabytes(self.total.unwrap_or(self.received)))
            }
            DownloadState::Failed => "Failed or cancelled".to_owned(),
        }
    }

    /// 0..1 when the size is known.
    pub fn fraction(&self) -> Option<f32> {
        let total = self.total.filter(|t| *t > 0)?;
        Some((self.received as f64 / total as f64).clamp(0.0, 1.0) as f32)
    }

    fn folder(&self) -> PathBuf {
        self.file.parent().map(PathBuf::from).unwrap_or_default()
    }
}

fn megabytes(bytes: u64) -> String {
    format!("{:.1} MB", bytes as f64 / 1_048_576.0)
}

pub fn is_safe_to_open(path: &std::path::Path) -> bool {
    path.extension()
        .and_then(|e| e.to_str())
        .is_some_and(|e| SAFE_TO_OPEN.contains(&e.to_ascii_lowercase().as_str()))
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn only_viewer_types_open_automatically() {
        assert!(is_safe_to_open(std::path::Path::new("a/Report.PDF")));
        assert!(is_safe_to_open(std::path::Path::new("clip.mp4")));
        for risky in [
            "setup.exe",
            "x.msi",
            "run.bat",
            "a.ps1",
            "doc.docm",
            "a.lnk",
            "a.js",
            "noext",
        ] {
            assert!(!is_safe_to_open(std::path::Path::new(risky)), "{risky}");
        }
    }

    #[test]
    fn tracks_progress_and_completion() {
        let mut list = Downloads::default();
        list.started(
            TabId(1),
            DownloadId(1),
            PathBuf::from("C:/d/a.zip"),
            Some(200),
        );
        list.progress(DownloadId(1), 50, None);
        assert_eq!(
            list.latest().map(Download::progress_label).as_deref(),
            Some("25%")
        );
        assert_eq!(list.active_count(), 1);
        list.finished(DownloadId(1), true);
        assert_eq!(list.active_count(), 0);
        assert_eq!(
            list.latest().map(Download::file_name).as_deref(),
            Some("a.zip")
        );
    }

    #[test]
    fn unknown_size_shows_megabytes() {
        let mut list = Downloads::default();
        list.started(TabId(1), DownloadId(2), PathBuf::from("b.bin"), None);
        list.progress(DownloadId(2), 3 * 1_048_576, None);
        assert_eq!(
            list.latest().map(Download::progress_label).as_deref(),
            Some("3.0 MB")
        );
    }
}
