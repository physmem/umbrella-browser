use std::path::PathBuf;

use serde::{Deserialize, Serialize};

use crate::ThemeId;
use crate::session::SessionError;
use crate::spring::MotionStyle;
use crate::store::KvStore;

const TABLE: &str = "settings";
const KEY: &str = "current";

pub const DEFAULT_SEARCH: &str = "https://duckduckgo.com/?q={query}";

/// User preferences. Every field has a default so older files keep loading.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
#[serde(default)]
pub struct Settings {
    /// `None` follows the Windows light/dark setting with Safari Glass.
    pub theme: Option<ThemeId>,
    pub motion: MotionStyle,
    pub reduced_glass: bool,
    pub search_template: String,
    pub vault_idle_lock_minutes: u32,
    /// Also wrap the vault key with Windows DPAPI so it unlocks without a
    /// password for this Windows user. Off by default.
    pub vault_dpapi: bool,
    pub block_ads: bool,
    pub strip_tracking: bool,
    pub start_page: StartPage,
    pub startup: Startup,
    /// Page zoom for new tabs and for "reset zoom".
    pub default_zoom: f32,
    pub tracking_prevention: TrackingLevel,
    /// Blocks windows a page opens without a click or key press.
    pub block_popups: bool,
    /// Show a toolbar notice when a page looks up your IP address.
    pub warn_ip_lookups: bool,
    pub downloads: DownloadPrefs,
    /// Browser-process switches. They only take effect after a restart.
    pub engine: EngineFlags,
    /// Unpacked extensions, loaded into every workspace's profile.
    pub extensions: Vec<ExtensionEntry>,
}

impl Default for Settings {
    fn default() -> Self {
        Self {
            theme: None,
            motion: MotionStyle::Calm,
            reduced_glass: false,
            search_template: DEFAULT_SEARCH.to_owned(),
            vault_idle_lock_minutes: 15,
            vault_dpapi: false,
            block_ads: true,
            strip_tracking: true,
            start_page: StartPage::default(),
            startup: Startup::Restore,
            default_zoom: crate::tab::DEFAULT_ZOOM,
            tracking_prevention: TrackingLevel::Balanced,
            block_popups: true,
            warn_ip_lookups: true,
            downloads: DownloadPrefs::default(),
            engine: EngineFlags::default(),
            extensions: Vec::new(),
        }
    }
}

impl Settings {
    pub fn load(kv: &KvStore) -> Result<Self, SessionError> {
        let Some(bytes) = kv.get(TABLE, KEY)? else {
            return Ok(Self::default());
        };
        Ok(serde_json::from_slice(&bytes)?)
    }

    pub fn save(&self, kv: &KvStore) -> Result<(), SessionError> {
        kv.put(TABLE, KEY, &serde_json::to_vec(self)?)?;
        Ok(())
    }
}

/// What a fresh launch shows.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "kebab-case")]
pub enum Startup {
    /// Bring back every tab from last time.
    Restore,
    /// Close last session's tabs and open one start page.
    StartPage,
}

/// WebView2's built-in tracking prevention, separate from our filter lists.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "kebab-case")]
pub enum TrackingLevel {
    Off,
    Basic,
    Balanced,
    Strict,
}

#[derive(Debug, Clone, Default, PartialEq, Eq, Serialize, Deserialize)]
#[serde(default)]
pub struct DownloadPrefs {
    /// `None` keeps the Windows Downloads folder.
    pub folder: Option<PathBuf>,
    /// Show a save dialog for every download.
    pub ask_where: bool,
    /// Open finished files whose type is safe to open automatically.
    pub open_when_done: bool,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct ExtensionEntry {
    pub folder: PathBuf,
    /// Empty until WebView2 has loaded it once.
    #[serde(default)]
    pub id: String,
    #[serde(default)]
    pub name: String,
    pub enabled: bool,
}

/// Switches passed to the browser process when it starts.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(default)]
pub struct EngineFlags {
    pub hardware_acceleration: bool,
    pub smooth_scrolling: bool,
    /// Let pages start playing sound without a click first.
    pub autoplay_sound: bool,
    /// Run the GPU and audio services inside the browser process: fewer
    /// entries in Task Manager, but one crash takes more down with it.
    pub fewer_processes: bool,
    /// Keep WebRTC to the default route, so it cannot reveal local
    /// addresses or the real IP behind a VPN.
    pub webrtc_protection: bool,
}

impl Default for EngineFlags {
    fn default() -> Self {
        Self {
            hardware_acceleration: true,
            smooth_scrolling: true,
            autoplay_sound: true,
            fewer_processes: false,
            webrtc_protection: true,
        }
    }
}

impl EngineFlags {
    /// Chromium switches for the flags that differ from its own defaults.
    pub fn browser_args(&self) -> Vec<&'static str> {
        let mut args = Vec::new();
        if !self.hardware_acceleration {
            args.push("--disable-gpu");
        }
        if !self.smooth_scrolling {
            args.push("--disable-smooth-scrolling");
        }
        if !self.autoplay_sound {
            args.push("--autoplay-policy=document-user-activation-required");
        }
        if self.fewer_processes {
            args.extend([
                "--in-process-gpu",
                "--disable-features=AudioServiceOutOfProcess",
            ]);
        }
        if self.webrtc_protection {
            args.push("--force-webrtc-ip-handling-policy=default_public_interface_only");
        }
        args
    }
}

/// A site pinned to the start page.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct Favorite {
    pub title: String,
    pub url: String,
}

impl Favorite {
    pub fn new(title: &str, url: &str) -> Self {
        Self {
            title: title.to_owned(),
            url: url.to_owned(),
        }
    }
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "kebab-case", tag = "kind", content = "value")]
pub enum StartBackground {
    /// The window's own glass.
    Glass,
    /// One of the built-in gradients, by index.
    Gradient(usize),
    /// A picture the user chose, copied into the data folder.
    Image(String),
}

/// Layout and content of the new-tab start page.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
#[serde(default)]
pub struct StartPage {
    pub favorites: Vec<Favorite>,
    pub show_favorites: bool,
    pub show_frequent: bool,
    pub background: StartBackground,
}

impl Default for StartPage {
    fn default() -> Self {
        Self {
            favorites: vec![
                Favorite::new("YouTube", "https://www.youtube.com/"),
                Favorite::new("Wikipedia", "https://en.wikipedia.org/"),
                Favorite::new("GitHub", "https://github.com/"),
                Favorite::new("Reddit", "https://www.reddit.com/"),
                Favorite::new("Maps", "https://www.google.com/maps"),
                Favorite::new("Gmail", "https://mail.google.com/"),
            ],
            show_favorites: true,
            show_frequent: true,
            background: StartBackground::Glass,
        }
    }
}

impl StartPage {
    pub fn is_pinned(&self, url: &str) -> bool {
        self.favorites.iter().any(|f| f.url == url)
    }

    /// Pins, or unpins if already there. Returns true when now pinned.
    pub fn toggle_pin(&mut self, title: &str, url: &str) -> bool {
        if let Some(i) = self.favorites.iter().position(|f| f.url == url) {
            self.favorites.remove(i);
            return false;
        }
        self.favorites.push(Favorite::new(title, url));
        true
    }
}

/// What the omnibox should do with typed text.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum OmniboxTarget {
    Url(String),
    Search(String),
}

impl OmniboxTarget {
    /// Treats input as a URL when it has a scheme or looks like a host
    /// (`example.com`, `localhost:3000`); anything else is a search.
    pub fn interpret(input: &str) -> Option<Self> {
        let text = input.trim();
        if text.is_empty() {
            return None;
        }
        if has_scheme(text) {
            return Some(Self::Url(text.to_owned()));
        }
        if let Some(name) = host_name(text) {
            // Local servers rarely have certificates; everything else gets https.
            let scheme = if is_local(name) { "http" } else { "https" };
            return Some(Self::Url(format!("{scheme}://{text}")));
        }
        Some(Self::Search(text.to_owned()))
    }

    pub fn into_url(self, search_template: &str) -> String {
        match self {
            Self::Url(url) => url,
            Self::Search(query) => search_template.replace("{query}", &encode_query(&query)),
        }
    }
}

fn has_scheme(text: &str) -> bool {
    ["http://", "https://", "file://", "about:", "edge://"]
        .iter()
        .any(|s| text.starts_with(s))
}

/// The host part when `text` looks like an address rather than a search.
fn host_name(text: &str) -> Option<&str> {
    if text.contains(char::is_whitespace) {
        return None;
    }
    let host = text.split(['/', '?', '#']).next().unwrap_or_default();
    let name = host.rsplit_once(':').map_or(host, |(h, port)| {
        if port.chars().all(|c| c.is_ascii_digit()) {
            h
        } else {
            host
        }
    });
    let plausible = name == "localhost"
        || (name.contains('.') && !name.starts_with('.') && !name.ends_with('.'));
    plausible.then_some(name)
}

fn is_local(name: &str) -> bool {
    name == "localhost"
        || name.ends_with(".localhost")
        || name.parse::<std::net::Ipv4Addr>().is_ok()
}

fn encode_query(query: &str) -> String {
    let mut out = String::with_capacity(query.len());
    for byte in query.bytes() {
        match byte {
            b'A'..=b'Z' | b'a'..=b'z' | b'0'..=b'9' | b'-' | b'_' | b'.' | b'~' => {
                out.push(byte as char)
            }
            b' ' => out.push('+'),
            other => out.push_str(&format!("%{other:02X}")),
        }
    }
    out
}

#[cfg(test)]
mod tests {
    use super::*;

    fn url(input: &str) -> Option<String> {
        OmniboxTarget::interpret(input).map(|t| t.into_url(DEFAULT_SEARCH))
    }

    #[test]
    fn hosts_become_https_urls() {
        assert_eq!(url("example.com"), Some("https://example.com".into()));
        assert_eq!(
            url("localhost:3000/x"),
            Some("http://localhost:3000/x".into())
        );
        assert_eq!(url("http://a.test"), Some("http://a.test".into()));
        assert_eq!(
            url("127.0.0.1:8765/a"),
            Some("http://127.0.0.1:8765/a".into())
        );
    }

    #[test]
    fn everything_else_searches() {
        assert_eq!(
            url("rust spring physics"),
            Some("https://duckduckgo.com/?q=rust+spring+physics".into())
        );
        assert_eq!(url("c++"), Some("https://duckduckgo.com/?q=c%2B%2B".into()));
        assert_eq!(
            url("version 1.2"),
            Some("https://duckduckgo.com/?q=version+1.2".into())
        );
        assert_eq!(url("   "), None);
    }

    #[test]
    fn pins_toggle() {
        let mut page = StartPage::default();
        assert!(page.is_pinned("https://github.com/"));
        assert!(!page.toggle_pin("GitHub", "https://github.com/"));
        assert!(!page.is_pinned("https://github.com/"));
        assert!(page.toggle_pin("Rust", "https://www.rust-lang.org/"));
        assert_eq!(
            page.favorites.last().map(|f| f.title.as_str()),
            Some("Rust")
        );
    }

    #[test]
    fn engine_flags_map_to_switches() {
        assert_eq!(
            EngineFlags::default().browser_args(),
            ["--force-webrtc-ip-handling-policy=default_public_interface_only"]
        );
        let lean = EngineFlags {
            hardware_acceleration: false,
            smooth_scrolling: false,
            autoplay_sound: false,
            fewer_processes: true,
            webrtc_protection: false,
        };
        assert_eq!(
            lean.browser_args(),
            [
                "--disable-gpu",
                "--disable-smooth-scrolling",
                "--autoplay-policy=document-user-activation-required",
                "--in-process-gpu",
                "--disable-features=AudioServiceOutOfProcess",
            ]
        );
    }

    #[test]
    fn older_settings_gain_new_defaults() -> Result<(), Box<dyn std::error::Error>> {
        let old: Settings = serde_json::from_str(r#"{"motion":"lively","block_ads":false}"#)?;
        assert!(!old.block_ads);
        assert!(old.block_popups);
        assert_eq!(old.tracking_prevention, TrackingLevel::Balanced);
        assert!(old.engine.hardware_acceleration);
        Ok(())
    }

    #[test]
    fn settings_round_trip_with_defaults() -> Result<(), Box<dyn std::error::Error>> {
        let dir = tempfile::tempdir()?;
        let kv = KvStore::open(&dir.path().join("p.redb"))?;
        assert_eq!(Settings::load(&kv)?, Settings::default());
        let lively = Settings {
            motion: MotionStyle::Lively,
            ..Settings::default()
        };
        lively.save(&kv)?;
        assert_eq!(Settings::load(&kv)?.motion, MotionStyle::Lively);
        Ok(())
    }
}
