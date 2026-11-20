//! Filter list downloads. Lists are fetched at runtime (never bundled),
//! written atomically, and refreshed when older than `MAX_AGE`.

use std::fs;
use std::io::{self, Read};
use std::path::{Path, PathBuf};
use std::time::{Duration, SystemTime};

use thiserror::Error;

pub const MAX_AGE: Duration = Duration::from_secs(4 * 24 * 60 * 60);
const MAX_BYTES: u64 = 16 * 1024 * 1024;
const TIMEOUT: Duration = Duration::from_secs(30);

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct ListSource {
    pub name: &'static str,
    pub url: &'static str,
}

pub const DEFAULT_LISTS: &[ListSource] = &[
    ListSource {
        name: "easylist",
        url: "https://easylist.to/easylist/easylist.txt",
    },
    ListSource {
        name: "easyprivacy",
        url: "https://easylist.to/easylist/easyprivacy.txt",
    },
];

#[derive(Debug, Error)]
pub enum ListError {
    #[error("download failed: {0}")]
    Http(#[from] ureq::Error),
    #[error("cannot write list: {0}")]
    Io(#[from] io::Error),
    #[error("response is not a filter list")]
    NotAList,
}

impl ListSource {
    pub fn path(&self, dir: &Path) -> PathBuf {
        dir.join(format!("{}.txt", self.name))
    }

    pub fn is_stale(&self, dir: &Path, now: SystemTime) -> bool {
        let modified = fs::metadata(self.path(dir)).and_then(|m| m.modified());
        match modified {
            Ok(time) => now.duration_since(time).is_ok_and(|age| age >= MAX_AGE),
            Err(_) => true,
        }
    }
}

/// Downloads every stale list (or all of them with `force`). Returns true
/// when at least one file changed, so the caller knows to rebuild.
pub fn refresh(dir: &Path, sources: &[ListSource], force: bool) -> (bool, Vec<ListError>) {
    let now = SystemTime::now();
    let mut changed = false;
    let mut errors = Vec::new();
    for source in sources.iter().filter(|s| force || s.is_stale(dir, now)) {
        match download(source, dir) {
            Ok(()) => changed = true,
            Err(e) => errors.push(e),
        }
    }
    (changed, errors)
}

fn download(source: &ListSource, dir: &Path) -> Result<(), ListError> {
    // Windows' own TLS stack (SChannel): no bundled crypto, system roots.
    let tls = ureq::tls::TlsConfig::builder()
        .provider(ureq::tls::TlsProvider::NativeTls)
        .root_certs(ureq::tls::RootCerts::PlatformVerifier)
        .build();
    let agent = ureq::Agent::config_builder()
        .timeout_global(Some(TIMEOUT))
        .tls_config(tls)
        .build()
        .new_agent();
    let mut response = agent.get(source.url).call()?;
    let mut body = Vec::new();
    response
        .body_mut()
        .as_reader()
        .take(MAX_BYTES)
        .read_to_end(&mut body)?;
    if !looks_like_list(&body) {
        return Err(ListError::NotAList);
    }
    write_atomic(&source.path(dir), &body)?;
    Ok(())
}

/// A captive portal or error page must not replace a good list.
fn looks_like_list(body: &[u8]) -> bool {
    let head = String::from_utf8_lossy(&body[..body.len().min(512)]);
    let first = head
        .lines()
        .map(str::trim)
        .find(|l| !l.is_empty())
        .unwrap_or_default();
    first.starts_with("[Adblock") || first.starts_with('!')
}

fn write_atomic(path: &Path, bytes: &[u8]) -> io::Result<()> {
    if let Some(dir) = path.parent() {
        fs::create_dir_all(dir)?;
    }
    let tmp = path.with_extension("txt.part");
    fs::write(&tmp, bytes)?;
    fs::rename(&tmp, path)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn recognizes_filter_lists() {
        assert!(looks_like_list(
            b"[Adblock Plus 2.0]\n! Title: EasyList\n||ads.example^"
        ));
        assert!(looks_like_list(b"\n! comment first\n"));
        assert!(!looks_like_list(
            b"<!doctype html><html>Sign in to Wi-Fi</html>"
        ));
        assert!(!looks_like_list(b""));
    }

    #[test]
    fn missing_or_old_lists_are_stale() -> io::Result<()> {
        let dir = tempfile::tempdir()?;
        let source = DEFAULT_LISTS[0];
        assert!(source.is_stale(dir.path(), SystemTime::now()));
        write_atomic(&source.path(dir.path()), b"! fresh")?;
        assert!(!source.is_stale(dir.path(), SystemTime::now()));
        assert!(source.is_stale(dir.path(), SystemTime::now() + MAX_AGE));
        Ok(())
    }
}
