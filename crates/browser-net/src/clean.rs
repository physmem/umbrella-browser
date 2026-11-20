use url::Url;

/// Exact query keys that only exist to track clicks across sites.
const TRACKING_KEYS: &[&str] = &[
    "fbclid",
    "gclid",
    "gclsrc",
    "dclid",
    "gbraid",
    "wbraid",
    "msclkid",
    "yclid",
    "twclid",
    "ttclid",
    "igshid",
    "li_fat_id",
    "mc_cid",
    "mc_eid",
    "_hsenc",
    "_hsmi",
    "mkt_tok",
    "oly_anon_id",
    "oly_enc_id",
    "vero_id",
    "rb_clickid",
    "s_cid",
];

const TRACKING_PREFIXES: &[&str] = &["utm_", "pk_", "hsa_"];

/// Returns the URL without tracking parameters, or `None` if nothing changed.
/// Only http(s) URLs are touched; everything else passes through.
pub fn strip_tracking(url: &str) -> Option<String> {
    let mut parsed = Url::parse(url).ok()?;
    if !matches!(parsed.scheme(), "http" | "https") || parsed.query().is_none() {
        return None;
    }
    let pairs: Vec<(String, String)> = parsed.query_pairs().into_owned().collect();
    let kept: Vec<&(String, String)> = pairs.iter().filter(|(k, _)| !is_tracking(k)).collect();
    if kept.len() == pairs.len() {
        return None;
    }
    if kept.is_empty() {
        parsed.set_query(None);
    } else {
        parsed.query_pairs_mut().clear().extend_pairs(kept);
    }
    Some(parsed.into())
}

fn is_tracking(key: &str) -> bool {
    let key = key.to_ascii_lowercase();
    TRACKING_KEYS.contains(&key.as_str()) || TRACKING_PREFIXES.iter().any(|p| key.starts_with(p))
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn strips_known_trackers_and_keeps_the_rest() {
        let url = "https://shop.test/item?id=7&utm_source=x&UTM_Medium=y&fbclid=abc#top";
        assert_eq!(
            strip_tracking(url).as_deref(),
            Some("https://shop.test/item?id=7#top")
        );
    }

    #[test]
    fn drops_empty_query() {
        let url = "https://news.test/a?gclid=1&utm_campaign=z";
        assert_eq!(strip_tracking(url).as_deref(), Some("https://news.test/a"));
    }

    #[test]
    fn leaves_clean_and_non_web_urls_alone() {
        assert_eq!(strip_tracking("https://a.test/?q=utm"), None);
        assert_eq!(strip_tracking("https://a.test/"), None);
        assert_eq!(strip_tracking("file:///C:/a.html?utm_source=x"), None);
    }
}
