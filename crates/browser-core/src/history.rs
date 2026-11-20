use serde::{Deserialize, Serialize};
use thiserror::Error;

use crate::fuzzy;
use crate::store::{KvStore, StoreError};

const HISTORY: &str = "history";
const BOOKMARKS: &str = "bookmarks";

#[derive(Debug, Error)]
pub enum HistoryError {
    #[error(transparent)]
    Store(#[from] StoreError),
    #[error("history data is corrupt: {0}")]
    Corrupt(#[from] serde_json::Error),
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct Visit {
    pub url: String,
    pub title: String,
    pub visits: u32,
    pub last_visit: u64,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct Bookmark {
    pub url: String,
    pub title: String,
    pub added: u64,
}

/// Browsing history, one record per URL. Lives in the profile database.
#[derive(Clone)]
pub struct History {
    kv: KvStore,
}

impl History {
    pub fn new(kv: KvStore) -> Self {
        Self { kv }
    }

    pub fn record_visit(&self, url: &str, title: &str, now: u64) -> Result<(), HistoryError> {
        let mut visit = self.get(url)?.unwrap_or_else(|| Visit {
            url: url.to_owned(),
            title: String::new(),
            visits: 0,
            last_visit: now,
        });
        visit.visits = visit.visits.saturating_add(1);
        visit.last_visit = now;
        if !title.is_empty() {
            visit.title = title.to_owned();
        }
        self.put(&visit)
    }

    /// Titles usually arrive after the visit was recorded.
    pub fn set_title(&self, url: &str, title: &str) -> Result<(), HistoryError> {
        let Some(mut visit) = self.get(url)? else {
            return Ok(());
        };
        visit.title = title.to_owned();
        self.put(&visit)
    }

    /// Most recent first.
    /// Most visited first, for the start page.
    pub fn frequent(&self, limit: usize) -> Result<Vec<Visit>, HistoryError> {
        let mut all = self.all()?;
        all.sort_by_key(|v| (std::cmp::Reverse(v.visits), std::cmp::Reverse(v.last_visit)));
        all.truncate(limit);
        Ok(all)
    }

    pub fn recent(&self, limit: usize) -> Result<Vec<Visit>, HistoryError> {
        let mut all = self.all()?;
        all.sort_by_key(|v| std::cmp::Reverse(v.last_visit));
        all.truncate(limit);
        Ok(all)
    }

    pub fn search(&self, query: &str, now: u64, limit: usize) -> Result<Vec<Visit>, HistoryError> {
        let mut scored: Vec<(i64, Visit)> = self
            .all()?
            .into_iter()
            .filter_map(|v| {
                let text = fuzzy::best(query, &[&v.title, &v.url])?;
                Some((i64::from(text) + frecency(&v, now), v))
            })
            .collect();
        scored.sort_by_key(|entry| std::cmp::Reverse(entry.0));
        Ok(scored.into_iter().take(limit).map(|(_, v)| v).collect())
    }

    pub fn clear(&self) -> Result<(), HistoryError> {
        Ok(self.kv.clear(HISTORY)?)
    }

    fn all(&self) -> Result<Vec<Visit>, HistoryError> {
        let mut out = Vec::new();
        for bytes in self.kv.values(HISTORY)? {
            out.push(serde_json::from_slice(&bytes)?);
        }
        Ok(out)
    }

    fn get(&self, url: &str) -> Result<Option<Visit>, HistoryError> {
        let Some(bytes) = self.kv.get(HISTORY, url)? else {
            return Ok(None);
        };
        Ok(Some(serde_json::from_slice(&bytes)?))
    }

    fn put(&self, visit: &Visit) -> Result<(), HistoryError> {
        Ok(self
            .kv
            .put(HISTORY, &visit.url, &serde_json::to_vec(visit)?)?)
    }
}

/// Frequent and recent pages float up; a visit a month old counts for little.
fn frecency(visit: &Visit, now: u64) -> i64 {
    let age_days = now.saturating_sub(visit.last_visit) / 86_400;
    let recency = 30i64.saturating_sub(age_days as i64).max(0);
    i64::from(visit.visits.min(50)) * 2 + recency
}

#[derive(Clone)]
pub struct Bookmarks {
    kv: KvStore,
}

impl Bookmarks {
    pub fn new(kv: KvStore) -> Self {
        Self { kv }
    }

    pub fn contains(&self, url: &str) -> Result<bool, HistoryError> {
        Ok(self.kv.get(BOOKMARKS, url)?.is_some())
    }

    /// Adds unless already there; returns true when it was new.
    pub fn add(&self, url: &str, title: &str, now: u64) -> Result<bool, HistoryError> {
        if self.contains(url)? {
            return Ok(false);
        }
        let mark = Bookmark {
            url: url.to_owned(),
            title: title.to_owned(),
            added: now,
        };
        self.kv.put(BOOKMARKS, url, &serde_json::to_vec(&mark)?)?;
        Ok(true)
    }

    /// Adds or removes; returns true when the page is now bookmarked.
    pub fn toggle(&self, url: &str, title: &str, now: u64) -> Result<bool, HistoryError> {
        if self.kv.delete(BOOKMARKS, url)? {
            return Ok(false);
        }
        let mark = Bookmark {
            url: url.to_owned(),
            title: title.to_owned(),
            added: now,
        };
        self.kv.put(BOOKMARKS, url, &serde_json::to_vec(&mark)?)?;
        Ok(true)
    }

    pub fn search(&self, query: &str, limit: usize) -> Result<Vec<Bookmark>, HistoryError> {
        let mut scored = Vec::new();
        for bytes in self.kv.values(BOOKMARKS)? {
            let mark: Bookmark = serde_json::from_slice(&bytes)?;
            if let Some(score) = fuzzy::best(query, &[&mark.title, &mark.url]) {
                scored.push((score, mark));
            }
        }
        scored.sort_by_key(|entry| std::cmp::Reverse(entry.0));
        Ok(scored.into_iter().take(limit).map(|(_, m)| m).collect())
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    type TestResult = Result<(), Box<dyn std::error::Error>>;

    fn store() -> Result<(tempfile::TempDir, KvStore), Box<dyn std::error::Error>> {
        let dir = tempfile::tempdir()?;
        let kv = KvStore::open(&dir.path().join("profile.redb"))?;
        Ok((dir, kv))
    }

    #[test]
    fn visits_accumulate_and_search_ranks_frequent_pages() -> TestResult {
        let (_dir, kv) = store()?;
        let history = History::new(kv);
        history.record_visit("https://docs.rs/redb", "", 1_000)?;
        history.set_title("https://docs.rs/redb", "redb - Rust")?;
        for _ in 0..5 {
            history.record_visit("https://doc.rust-lang.org/", "Rust Docs", 1_000)?;
        }
        history.record_visit("https://late.test/", "Late", 2_000)?;
        assert_eq!(history.recent(1)?[0].url, "https://late.test/");
        let hits = history.search("rust", 1_000, 10)?;
        assert_eq!(hits.len(), 2);
        assert_eq!(hits[0].url, "https://doc.rust-lang.org/");
        assert_eq!(hits[0].visits, 5);
        assert_eq!(hits[1].title, "redb - Rust");
        history.clear()?;
        assert!(history.search("", 1_000, 10)?.is_empty());
        Ok(())
    }

    #[test]
    fn bookmarks_toggle() -> TestResult {
        let (_dir, kv) = store()?;
        let marks = Bookmarks::new(kv);
        assert!(marks.toggle("https://a.test/", "A", 1)?);
        assert!(marks.contains("https://a.test/")?);
        assert_eq!(marks.search("a", 5)?.len(), 1);
        assert!(!marks.toggle("https://a.test/", "A", 2)?);
        assert!(!marks.contains("https://a.test/")?);
        Ok(())
    }
}
