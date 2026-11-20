//! Who learned your IP address while a page loaded.
//!
//! Every server a page makes the browser contact sees the IP address; no
//! browser can see what a server does with it. What can be seen is which
//! other sites were contacted, and whether the page asked an IP-lookup
//! service where you are, which only makes sense if it wants your IP.

use std::collections::BTreeSet;

/// Services whose job is turning an IP address into an address or location.
const IP_LOOKUP_HOSTS: &[&str] = &[
    "ipify.org",
    "ipinfo.io",
    "ipapi.co",
    "ipapi.com",
    "ip-api.com",
    "ipwho.is",
    "ipwhois.app",
    "ipgeolocation.io",
    "ipdata.co",
    "ipregistry.co",
    "ipstack.com",
    "db-ip.com",
    "geojs.io",
    "geolocation-db.com",
    "geoip-db.com",
    "freegeoip.app",
    "extreme-ip-lookup.com",
    "seeip.org",
    "ifconfig.me",
    "icanhazip.com",
    "checkip.amazonaws.com",
    "geoip.maxmind.com",
    "geolocation.onetrust.com",
    "ip.nf",
    "myip.com",
    "api.myip.com",
    "wtfismyip.com",
];

/// Second-level labels used under country codes, so `bbc.co.uk` is one site.
const SHARED_SECOND_LEVEL: &[&str] = &["co", "com", "org", "net", "gov", "ac", "edu", "ne", "or"];

#[derive(Debug, Clone, Default, PartialEq, Eq)]
pub struct Exposure {
    /// Other sites the page made the browser contact; each saw the IP.
    pub third_parties: BTreeSet<String>,
    /// IP-lookup services among them.
    pub ip_lookups: BTreeSet<String>,
    /// Requests the blocker stopped before they left the machine.
    pub blocked: u32,
    /// The page opened a WebRTC connection, which can reveal the local and
    /// real public address even behind a VPN.
    pub webrtc: bool,
}

impl Exposure {
    /// Records one request from `page`. Blocked requests never reached the
    /// server, so they do not count as exposure.
    pub fn record(&mut self, url: &str, page: &str, blocked: bool) {
        if blocked {
            self.blocked += 1;
            return;
        }
        let (Some(host), Some(page_host)) = (host_of(url), host_of(page)) else {
            return;
        };
        let site = site_of(&host);
        if site == site_of(&page_host) {
            return;
        }
        if is_ip_lookup(&host) {
            self.ip_lookups.insert(host);
        }
        self.third_parties.insert(site);
    }

    /// Signals that point at the page wanting the IP itself, not just
    /// loading content from elsewhere.
    pub fn looks_up_ip(&self) -> bool {
        !self.ip_lookups.is_empty() || self.webrtc
    }
}

pub fn is_ip_lookup(host: &str) -> bool {
    IP_LOOKUP_HOSTS
        .iter()
        .any(|known| host == *known || host.ends_with(&format!(".{known}")))
}

/// `www.news.bbc.co.uk` -> `bbc.co.uk`, `cdn.example.com` -> `example.com`.
/// Close enough for grouping; it is not the full public suffix list.
pub fn site_of(host: &str) -> String {
    let labels: Vec<&str> = host.trim_end_matches('.').split('.').collect();
    if labels.len() <= 2 || host.parse::<std::net::IpAddr>().is_ok() {
        return host.to_owned();
    }
    let n = labels.len();
    let country = labels[n - 1].len() == 2;
    let take = if country && SHARED_SECOND_LEVEL.contains(&labels[n - 2]) {
        3
    } else {
        2
    };
    labels[n - take..].join(".")
}

fn host_of(url: &str) -> Option<String> {
    let parsed = url::Url::parse(url).ok()?;
    if !matches!(parsed.scheme(), "http" | "https" | "ws" | "wss") {
        return None;
    }
    parsed.host_str().map(|h| h.to_ascii_lowercase())
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn groups_hosts_into_sites() {
        assert_eq!(site_of("cdn.example.com"), "example.com");
        assert_eq!(site_of("www.news.bbc.co.uk"), "bbc.co.uk");
        assert_eq!(site_of("localhost"), "localhost");
        assert_eq!(site_of("127.0.0.1"), "127.0.0.1");
    }

    #[test]
    fn records_third_parties_and_lookups() {
        let mut e = Exposure::default();
        let page = "https://www.shop.example/";
        e.record("https://static.shop.example/app.js", page, false);
        e.record("https://www.google-analytics.com/collect", page, true);
        e.record("https://api.ipify.org/?format=json", page, false);
        e.record("https://fonts.gstatic.com/x.woff2", page, false);
        assert_eq!(e.blocked, 1);
        assert!(e.looks_up_ip());
        assert_eq!(
            e.third_parties.iter().cloned().collect::<Vec<_>>(),
            ["gstatic.com", "ipify.org"]
        );
        assert!(e.ip_lookups.contains("api.ipify.org"));
    }

    #[test]
    fn first_party_and_blocked_requests_are_not_exposure() {
        let mut e = Exposure::default();
        e.record("https://a.example.com/x", "https://example.com/", false);
        e.record("https://ipinfo.io/json", "https://example.com/", true);
        assert!(e.third_parties.is_empty());
        assert!(!e.looks_up_ip());
    }
}
