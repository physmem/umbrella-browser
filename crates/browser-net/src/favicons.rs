//! On-disk favicon cache keyed by host, so restored tabs show their icon
//! before the page has loaded.

use std::fs;
use std::io;
use std::path::{Path, PathBuf};

const MAX_BYTES: usize = 256 * 1024;
const PNG_MAGIC: &[u8] = b"\x89PNG\r\n\x1a\n";

#[derive(Debug, Clone)]
pub struct FaviconCache {
    dir: PathBuf,
}

impl FaviconCache {
    pub fn new(dir: impl Into<PathBuf>) -> Self {
        Self { dir: dir.into() }
    }

    pub fn get(&self, host: &str) -> Option<Vec<u8>> {
        let bytes = fs::read(self.path(host)).ok()?;
        is_png(&bytes).then_some(bytes)
    }

    /// Stores a PNG; anything else (or anything huge) is ignored, and an
    /// empty icon removes the cached one.
    pub fn put(&self, host: &str, png: &[u8]) -> io::Result<()> {
        if png.is_empty() {
            return match fs::remove_file(self.path(host)) {
                Err(e) if e.kind() != io::ErrorKind::NotFound => Err(e),
                _ => Ok(()),
            };
        }
        if !is_png(png) || png.len() > MAX_BYTES {
            return Ok(());
        }
        fs::create_dir_all(&self.dir)?;
        let tmp = self.path(host).with_extension("part");
        fs::write(&tmp, png)?;
        fs::rename(tmp, self.path(host))
    }

    fn path(&self, host: &str) -> PathBuf {
        self.dir.join(format!(
            "{:016x}.png",
            fnv1a(host.to_ascii_lowercase().as_bytes())
        ))
    }

    pub fn dir(&self) -> &Path {
        &self.dir
    }
}

fn is_png(bytes: &[u8]) -> bool {
    bytes.starts_with(PNG_MAGIC)
}

fn fnv1a(bytes: &[u8]) -> u64 {
    bytes.iter().fold(0xcbf2_9ce4_8422_2325, |h, b| {
        (h ^ u64::from(*b)).wrapping_mul(0x0100_0000_01b3)
    })
}

#[cfg(test)]
mod tests {
    use super::*;

    fn png() -> Vec<u8> {
        let mut bytes = PNG_MAGIC.to_vec();
        bytes.extend_from_slice(b"rest-of-image");
        bytes
    }

    #[test]
    fn stores_and_returns_png_by_host() -> io::Result<()> {
        let dir = tempfile::tempdir()?;
        let cache = FaviconCache::new(dir.path().join("icons"));
        assert!(cache.get("github.com").is_none());
        cache.put("GitHub.com", &png())?;
        assert_eq!(cache.get("github.com"), Some(png()));
        cache.put("github.com", &[])?;
        assert!(cache.get("github.com").is_none());
        Ok(())
    }

    #[test]
    fn ignores_non_png_and_oversized() -> io::Result<()> {
        let dir = tempfile::tempdir()?;
        let cache = FaviconCache::new(dir.path());
        cache.put("a.test", b"<html>not an image</html>")?;
        assert!(cache.get("a.test").is_none());
        let mut huge = png();
        huge.resize(MAX_BYTES + 1, 0);
        cache.put("b.test", &huge)?;
        assert!(cache.get("b.test").is_none());
        Ok(())
    }
}
