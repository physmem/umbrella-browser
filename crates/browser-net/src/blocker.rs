use std::fs;
use std::path::Path;

use adblock::lists::ParseOptions;
use adblock::request::Request;
use adblock::{Engine, FilterSet};

/// Request types as adblock filter lists name them.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum ResourceKind {
    Document,
    Subdocument,
    Stylesheet,
    Script,
    Image,
    Font,
    Media,
    Xhr,
    WebSocket,
    Ping,
    Other,
}

impl ResourceKind {
    fn as_filter_type(self) -> &'static str {
        match self {
            ResourceKind::Document => "document",
            ResourceKind::Subdocument => "subdocument",
            ResourceKind::Stylesheet => "stylesheet",
            ResourceKind::Script => "script",
            ResourceKind::Image => "image",
            ResourceKind::Font => "font",
            ResourceKind::Media => "media",
            ResourceKind::Xhr => "xmlhttprequest",
            ResourceKind::WebSocket => "websocket",
            ResourceKind::Ping => "ping",
            ResourceKind::Other => "other",
        }
    }
}

/// Network filter over EasyList-style rules. Built once per list change;
/// matching runs on every subresource request, so it stays allocation-light.
pub struct Blocker {
    engine: Engine,
    rule_count: usize,
}

impl Blocker {
    pub fn empty() -> Self {
        Self::from_rules(std::iter::empty::<String>())
    }

    pub fn from_rules<I, S>(rules: I) -> Self
    where
        I: IntoIterator<Item = S>,
        S: AsRef<str>,
    {
        let rules: Vec<String> = rules.into_iter().map(|r| r.as_ref().to_owned()).collect();
        let rule_count = rules.iter().filter(|r| is_rule(r)).count();
        let mut set = FilterSet::new(false);
        set.add_filter_list(
            rules.join(
                "
",
            ),
            ParseOptions::default(),
        );
        Self {
            engine: Engine::new_with_filter_set(set),
            rule_count,
        }
    }

    /// Loads every `.txt` list in `dir`. A missing folder means no blocking yet.
    pub fn load_dir(dir: &Path) -> std::io::Result<Self> {
        let entries = match fs::read_dir(dir) {
            Ok(entries) => entries,
            Err(e) if e.kind() == std::io::ErrorKind::NotFound => return Ok(Self::empty()),
            Err(e) => return Err(e),
        };
        let mut lines = Vec::new();
        for entry in entries {
            let path = entry?.path();
            if path.extension().is_some_and(|e| e == "txt") {
                lines.extend(fs::read_to_string(&path)?.lines().map(str::to_owned));
            }
        }
        Ok(Self::from_rules(lines))
    }

    /// Like `load_dir`, but reuses a compiled engine from `cache` when the
    /// lists have not changed. Parsing EasyList takes a few hundred ms; the
    /// cached engine loads in a few. A stale or corrupt cache is rebuilt.
    pub fn load_cached(dir: &Path, cache: &Path) -> std::io::Result<Self> {
        let key = lists_fingerprint(dir)?;
        if let Some(blocker) = read_cache(cache, key) {
            return Ok(blocker);
        }
        let blocker = Self::load_dir(dir)?;
        // A failed cache write only costs the next startup a reparse.
        let _ = write_cache(cache, key, &blocker);
        Ok(blocker)
    }

    pub fn rule_count(&self) -> usize {
        self.rule_count
    }

    /// Top-level documents are never blocked here: a blocked navigation reads
    /// as a broken browser, not a blocked ad.
    pub fn should_block(&self, url: &str, source_url: &str, kind: ResourceKind) -> bool {
        if kind == ResourceKind::Document || self.rule_count == 0 {
            return false;
        }
        let Ok(request) = Request::new(url, source_url, kind.as_filter_type(), "GET") else {
            return false;
        };
        self.engine.check_network_request(&request).should_block()
    }
}

const CACHE_MAGIC: &[u8; 8] = b"UMBRAB01";

/// Names, sizes and modification times of the list files; any edit or
/// download changes it.
fn lists_fingerprint(dir: &Path) -> std::io::Result<u64> {
    let mut files: Vec<(String, u64, u64)> = Vec::new();
    let entries = match fs::read_dir(dir) {
        Ok(entries) => entries,
        Err(e) if e.kind() == std::io::ErrorKind::NotFound => return Ok(0),
        Err(e) => return Err(e),
    };
    for entry in entries {
        let entry = entry?;
        let path = entry.path();
        if path.extension().is_none_or(|e| e != "txt") {
            continue;
        }
        let meta = entry.metadata()?;
        let modified = meta
            .modified()
            .ok()
            .and_then(|t| t.duration_since(std::time::UNIX_EPOCH).ok())
            .map_or(0, |d| d.as_secs());
        files.push((
            entry.file_name().to_string_lossy().into_owned(),
            meta.len(),
            modified,
        ));
    }
    files.sort();
    let text = format!("{files:?}");
    Ok(text.bytes().fold(0xcbf2_9ce4_8422_2325, |h, b| {
        (h ^ u64::from(b)).wrapping_mul(0x0100_0000_01b3)
    }))
}

fn read_cache(cache: &Path, key: u64) -> Option<Blocker> {
    let bytes = fs::read(cache).ok()?;
    let (magic, rest) = bytes.split_at_checked(8)?;
    let (stored_key, rest) = rest.split_at_checked(8)?;
    let (count, data) = rest.split_at_checked(8)?;
    if magic != CACHE_MAGIC || u64::from_le_bytes(stored_key.try_into().ok()?) != key {
        return None;
    }
    let mut engine = Engine::default();
    engine.deserialize(data).ok()?;
    let rule_count = usize::try_from(u64::from_le_bytes(count.try_into().ok()?)).ok()?;
    Some(Blocker { engine, rule_count })
}

fn write_cache(cache: &Path, key: u64, blocker: &Blocker) -> std::io::Result<()> {
    let data = blocker.engine.serialize();
    let mut out = Vec::with_capacity(24 + data.len());
    out.extend_from_slice(CACHE_MAGIC);
    out.extend_from_slice(&key.to_le_bytes());
    out.extend_from_slice(&(blocker.rule_count as u64).to_le_bytes());
    out.extend_from_slice(&data);
    let tmp = cache.with_extension("part");
    fs::write(&tmp, out)?;
    fs::rename(tmp, cache)
}

fn is_rule(line: &str) -> bool {
    let line = line.trim();
    !line.is_empty() && !line.starts_with('!') && !line.starts_with('[')
}

#[cfg(test)]
mod tests {
    use super::*;

    const RULES: &[&str] = &[
        "! comment",
        "||ads.example^",
        "||cdn.test/track.js$script,third-party",
        "@@||ads.example/allowed.png",
    ];

    #[test]
    fn blocks_matching_subresources() {
        let blocker = Blocker::from_rules(RULES);
        assert_eq!(blocker.rule_count(), 3);
        let page = "https://news.test/";
        assert!(blocker.should_block("https://ads.example/banner.png", page, ResourceKind::Image));
        assert!(blocker.should_block("https://cdn.test/track.js", page, ResourceKind::Script));
        assert!(!blocker.should_block("https://cdn.test/app.js", page, ResourceKind::Script));
    }

    #[test]
    fn honors_exceptions_and_never_blocks_documents() {
        let blocker = Blocker::from_rules(RULES);
        let page = "https://news.test/";
        assert!(!blocker.should_block(
            "https://ads.example/allowed.png",
            page,
            ResourceKind::Image
        ));
        assert!(!blocker.should_block("https://ads.example/", page, ResourceKind::Document));
    }

    #[test]
    fn first_party_option_respected() {
        let blocker = Blocker::from_rules(RULES);
        let own = "https://cdn.test/";
        assert!(!blocker.should_block("https://cdn.test/track.js", own, ResourceKind::Script));
    }

    #[test]
    fn missing_dir_blocks_nothing() -> std::io::Result<()> {
        let blocker = Blocker::load_dir(Path::new("no/such/filters"))?;
        assert!(!blocker.should_block(
            "https://ads.example/",
            "https://a.test/",
            ResourceKind::Image
        ));
        Ok(())
    }

    #[test]
    fn cache_round_trips_and_invalidates() -> std::io::Result<()> {
        let dir = tempfile::tempdir()?;
        let lists = dir.path().join("filters");
        fs::create_dir_all(&lists)?;
        let cache = dir.path().join("adblock.bin");
        fs::write(
            lists.join("easy.txt"),
            "||ads.example^
",
        )?;
        let first = Blocker::load_cached(&lists, &cache)?;
        assert!(cache.exists());
        let cached = Blocker::load_cached(&lists, &cache)?;
        assert_eq!(cached.rule_count(), first.rule_count());
        let page = "https://news.test/";
        assert!(cached.should_block("https://ads.example/x.png", page, ResourceKind::Image));
        fs::write(
            lists.join("more.txt"),
            "||track.example^
||pixel.example^
",
        )?;
        assert_eq!(Blocker::load_cached(&lists, &cache)?.rule_count(), 3);
        Ok(())
    }

    #[test]
    fn engine_is_send() {
        fn assert_send<T: Send>() {}
        assert_send::<Blocker>();
    }

    #[test]
    fn loads_txt_lists_from_dir() -> std::io::Result<()> {
        let dir = tempfile::tempdir()?;
        fs::write(dir.path().join("easy.txt"), "||ads.example^\n")?;
        fs::write(dir.path().join("notes.md"), "||news.test^\n")?;
        let blocker = Blocker::load_dir(dir.path())?;
        assert_eq!(blocker.rule_count(), 1);
        Ok(())
    }
}
