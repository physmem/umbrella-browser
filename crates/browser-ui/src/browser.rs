//! Browser state and behavior: which tabs exist, which have a live WebView2,
//! persistence and shortcuts. Rendering lives in `chrome.rs`.

use std::cell::{Cell, RefCell};
use std::collections::HashMap;
use std::path::PathBuf;
use std::rc::Rc;
use std::sync::Arc;
use std::time::{Duration, Instant, SystemTime, UNIX_EPOCH};

use browser_core::debounce::Debouncer;
use browser_core::history::{Bookmarks, History};
use browser_core::permissions::{PermissionKind, SitePermissions};
use browser_core::session::{Session, SessionError, SessionStore};
use browser_core::settings::{EngineFlags, OmniboxTarget, Settings, Startup};
use browser_core::spring::SpringParams;
use browser_core::store::{KvStore, StoreError};
use browser_core::tab::DEFAULT_ZOOM;
use browser_core::theme::{ThemeRegistry, ThemeTokens, presets};
use browser_core::{SavedTab, TabId, Workspace};
use browser_engine::{
    DownloadTarget, EngineEvent, EventQueue, HibernateAction, HibernatePolicy, LogicalRect,
    PageSettings, Profile, ProfileOptions, Residency, Shortcut, TabActivity, TabEvent, TabHooks,
    TabView,
};
use browser_net::{Blocker, Exposure, FaviconCache, ScriptLibrary, lists};
use futures::StreamExt;
use futures::channel::mpsc;
use gpui::{
    AnyWindowHandle, AppContext, Context, Entity, FocusHandle, Focusable, Subscription, Task,
    Window,
};
use gpui_ce_elements::editable_text::{EditableTextState, StringStorage, TextChanged};
use thiserror::Error;
use windows::Win32::Foundation::HWND;

use crate::downloads_ui::Downloads;
use crate::instance::Incoming;
use crate::motion::{Springs, TabMotion};
use crate::paths::Paths;
use crate::platform;
use crate::settings_ui::SettingsPane;
use crate::style::{self, ChromeColors};
use crate::suggest::Suggestion;
use crate::vault_ui::VaultUi;

pub const NEW_TAB_URL: &str = "about:blank";

/// The Settings page's place in the tab strip. It is not a saved tab, so
/// it never reaches the session; real ids never get this high.
pub(crate) const SETTINGS_TAB: TabId = TabId(u64::MAX);
/// Gap between split panes; the window backdrop shows through it.
pub(crate) const SPLIT_GUTTER: f32 = 8.0;
const ZOOM_STEP: f32 = 0.1;
const SAVE_TICK: Duration = Duration::from_millis(250);
const HIBERNATE_TICK: Duration = Duration::from_secs(30);

#[derive(Debug, Error)]
pub enum StartError {
    #[error("cannot create data folders: {0}")]
    Io(#[from] std::io::Error),
    #[error(transparent)]
    Store(#[from] StoreError),
    #[error(transparent)]
    Session(#[from] SessionError),
    #[error("the window has no native handle")]
    NoWindow,
}

/// A tab that has, or is getting, a WebView2 controller.
struct LiveTab {
    view: Option<TabView>,
    residency: Residency,
    visible: bool,
    hidden_since: Option<Instant>,
    playing_audio: bool,
    loading: bool,
}

impl LiveTab {
    fn pending() -> Self {
        Self {
            view: None,
            residency: Residency::Live,
            visible: false,
            hidden_since: Some(Instant::now()),
            playing_audio: false,
            loading: true,
        }
    }
}

pub struct Browser {
    pub(crate) paths: Paths,
    pub(crate) hwnd: HWND,
    pub(crate) window: AnyWindowHandle,
    kv: KvStore,
    store: SessionStore,
    pub(crate) session: Session,
    pub(crate) history: History,
    pub(crate) bookmarks: Bookmarks,
    pub(crate) permissions: SitePermissions,
    /// Permission requests waiting on the user, oldest first.
    pub(crate) permission_prompts: Vec<PermissionPrompt>,
    /// Results of extension installs, applied on the next pump.
    pub(crate) extension_inbox: Rc<RefCell<Vec<crate::extensions_ui::ExtensionResult>>>,
    pub(crate) extension_status: Option<String>,
    /// A short message in the toolbar and when it appeared.
    pub(crate) toast: Option<(String, Instant)>,
    pub(crate) import_status: Option<String>,
    /// Cookies waiting for a tab's page to go live before they are set.
    pub(crate) pending_cookies: Option<(TabId, Vec<browser_engine::ImportCookie>)>,
    /// After a profile-wide cookie import, the site to open signed in.
    pub(crate) pending_cookie_site: Option<String>,
    /// Results from in-page scripts (reader mode), applied on the next pump.
    pub(crate) script_inbox: Rc<RefCell<Vec<String>>>,
    /// The current profile has been brought in line with the settings list.
    pub(crate) extensions_synced: bool,
    pub(crate) settings: Settings,
    pub(crate) themes: ThemeRegistry,
    pub(crate) theme: ThemeTokens,
    pub(crate) colors: ChromeColors,
    pub(crate) motion: TabMotion,
    /// One-off animations (panels, pills, toggles) keyed by name.
    pub(crate) springs: RefCell<Springs>,
    pub(crate) omnibox: Entity<EditableTextState>,
    pub(crate) find_field: Entity<EditableTextState>,
    /// Theme editor state: the open colour role, its fields and sliders.
    pub(crate) theme_edit_role: Option<crate::theme_editor::ColorRole>,
    /// The picker's hue, kept apart so greys (which have none) don't lose it.
    pub(crate) edit_hue: f32,
    pub(crate) theme_name_field: Entity<EditableTextState>,
    pub(crate) theme_hex_field: Entity<EditableTextState>,
    pub(crate) slider_drag: Option<crate::theme_editor::Slider>,
    pub(crate) slider_bounds: crate::theme_editor::SliderBounds,
    pub(crate) find_open: bool,
    /// (active match, match count) for the find bar.
    pub(crate) find_result: (u32, u32),
    pub(crate) find_case: bool,
    pub(crate) focus: FocusHandle,
    pub(crate) vault: VaultUi,
    pub(crate) suggestions: Vec<Suggestion>,
    pub(crate) selected: usize,
    /// The field must be rendered before it can hold focus, so editing is
    /// tracked here rather than derived from focus.
    pub(crate) omnibox_editing: bool,
    /// Keyboard focus (Ctrl+L) selects the address to replace it; a click
    /// puts the caret at the end to edit it.
    omnibox_select_all: bool,
    /// The chrome button under the pointer, for animated hover fills.
    pub(crate) hovered: Option<String>,
    /// The tab under the pointer and when it got there, for the hover card.
    pub(crate) hovered_tab: Option<(TabId, Instant)>,
    pub(crate) tab_memory: Rc<RefCell<HashMap<TabId, MemoryReading>>>,
    memory_checked: Option<Instant>,
    pub(crate) page_info_open: bool,
    pub(crate) page_info: Option<crate::page_info::PageInfo>,
    /// Who saw the IP address while each tab's page loaded.
    pub(crate) exposure: Rc<RefCell<HashMap<TabId, Exposure>>>,
    /// The corner notice for a page that looked up the IP, and when it rose.
    pub(crate) ip_alert: Option<(TabId, Instant)>,
    /// Tabs already warned about for their current page.
    ip_alerted: Vec<TabId>,
    pub(crate) omnibox_span: (f32, f32),
    pub(crate) split: Option<TabId>,
    pub(crate) split_ratio: f32,
    pub(crate) split_dragging: bool,
    /// A page asked for fullscreen; the chrome is hidden and the window fills the screen.
    pub(crate) page_fullscreen: bool,
    pub(crate) settings_open: bool,
    pub(crate) settings_pane: SettingsPane,
    /// Where the settings column is scrolling to; a spring carries it there.
    pub(crate) settings_scroll: f32,
    /// Viewport and content heights from the last layout, for clamping.
    pub(crate) settings_metrics: Rc<Cell<(f32, f32)>>,
    /// While the scrollbar thumb is held: pointer-to-thumb-top offset.
    pub(crate) settings_thumb_grab: Option<f32>,
    pub(crate) customize_open: bool,
    pub(crate) downloads_open: bool,
    /// The last pop-up a page tried to open without a click.
    pub(crate) blocked_popup: Option<(TabId, String)>,
    /// Browser-process switches the running WebView2 was started with.
    launched_engine: EngineFlags,
    site_data: Rc<Cell<SiteData>>,
    profile_failures: Rc<Cell<u32>>,
    profile_starting: Rc<Cell<bool>>,
    pub(crate) downloads: Downloads,
    adblock_on: Rc<Cell<bool>>,
    content_rect: Option<(LogicalRect, f32)>,
    _subscriptions: Vec<Subscription>,
    pub(crate) tab_drag: Option<TabDrag>,
    pub(crate) tab_width: f32,
    pub(crate) preview: Option<Arc<gpui::Image>>,
    favicons: FaviconCache,
    icons: RefCell<HashMap<String, Option<Arc<gpui::Image>>>>,
    preview_bytes: Rc<RefCell<Option<Vec<u8>>>>,
    pub(crate) reduced_motion: bool,
    pub(crate) last_frame: Option<Instant>,
    queue: EventQueue,
    wake: mpsc::UnboundedSender<()>,
    profile: Rc<RefCell<Option<Profile>>>,
    arrivals: Rc<RefCell<Vec<TabView>>>,
    live: HashMap<TabId, LiveTab>,
    blocked: Rc<RefCell<HashMap<TabId, u32>>>,
    /// Swapped in from a background thread once the lists are compiled.
    blocker: Rc<RefCell<Arc<Blocker>>>,
    scripts: Vec<String>,
    debounce: Debouncer,
    cycle: Option<usize>,
    _tasks: Vec<Task<()>>,
}

impl Browser {
    pub fn new(
        window: &mut Window,
        cx: &mut Context<Self>,
        incoming: Incoming,
    ) -> Result<Self, StartError> {
        let paths = Paths::resolve();
        paths.ensure()?;
        let hwnd = platform::hwnd(window).ok_or(StartError::NoWindow)?;
        let kv = KvStore::open(&paths.profile_db())?;
        let store = SessionStore::with_store(kv.clone());
        let session = store.load()?.unwrap_or_default();
        let settings = Settings::load(&kv)?;
        let mut themes = ThemeRegistry::with_presets();
        for e in themes.load_dir(&paths.themes()) {
            eprintln!("theme {} skipped: {}", e.path.display(), e.error);
        }
        let (wake, rx) = mpsc::unbounded();
        let queue = EventQueue::new();
        let waker = wake.clone();
        queue.set_waker(move || {
            let _ = waker.unbounded_send(());
        });
        let omnibox = cx.new(|cx| EditableTextState::new(StringStorage::default(), cx));
        let find_field = cx.new(|cx| EditableTextState::new(StringStorage::default(), cx));
        let theme_name_field = cx.new(|cx| EditableTextState::new(StringStorage::default(), cx));
        let theme_hex_field = cx.new(|cx| EditableTextState::new(StringStorage::default(), cx));
        let theme = presets::safari_glass_dark();
        let vault_path = paths.vault();
        let favicon_dir = paths.favicons();
        let mut browser = Self {
            hwnd,
            window: window.window_handle(),
            history: History::new(kv.clone()),
            permissions: SitePermissions::new(kv.clone()),
            permission_prompts: Vec::new(),
            extension_inbox: Rc::default(),
            extension_status: None,
            toast: None,
            import_status: None,
            pending_cookies: None,
            pending_cookie_site: None,
            script_inbox: Rc::default(),
            extensions_synced: false,
            bookmarks: Bookmarks::new(kv.clone()),
            colors: ChromeColors::from_theme(&theme),
            motion: TabMotion::new(theme.motion),
            springs: RefCell::new(Springs::new(theme.motion)),
            theme,
            blocker: Rc::new(RefCell::new(Arc::new(Blocker::empty()))),
            scripts: load_scripts(&paths),
            paths,
            kv,
            store,
            session,
            settings,
            themes,
            omnibox,
            find_field,
            theme_edit_role: None,
            edit_hue: 210.0,
            theme_name_field,
            theme_hex_field,
            slider_drag: None,
            slider_bounds: Rc::default(),
            find_open: false,
            find_result: (0, 0),
            find_case: false,
            focus: cx.focus_handle(),
            vault: VaultUi::new(vault_path, cx.focus_handle()),
            suggestions: Vec::new(),
            selected: 0,
            omnibox_editing: false,
            omnibox_select_all: true,
            hovered: None,
            hovered_tab: None,
            tab_memory: Rc::default(),
            memory_checked: None,
            page_info_open: false,
            page_info: None,
            exposure: Rc::default(),
            ip_alert: None,
            ip_alerted: Vec::new(),
            omnibox_span: (0.0, 0.0),
            split: None,
            split_ratio: 0.5,
            split_dragging: false,
            page_fullscreen: false,
            settings_open: false,
            settings_pane: SettingsPane::default(),
            settings_scroll: 0.0,
            settings_metrics: Rc::default(),
            settings_thumb_grab: None,
            customize_open: false,
            downloads_open: false,
            blocked_popup: None,
            launched_engine: EngineFlags::default(),
            site_data: Rc::default(),
            profile_failures: Rc::default(),
            profile_starting: Rc::default(),
            downloads: Downloads::default(),
            adblock_on: Rc::new(Cell::new(true)),
            content_rect: None,
            _subscriptions: Vec::new(),
            tab_drag: None,
            tab_width: 0.0,
            preview: None,
            favicons: FaviconCache::new(favicon_dir),
            icons: RefCell::default(),
            preview_bytes: Rc::default(),
            reduced_motion: platform::reduced_motion(),
            last_frame: None,
            queue,
            wake,
            profile: Rc::default(),
            arrivals: Rc::default(),
            live: HashMap::new(),
            blocked: Rc::default(),
            debounce: Debouncer::default(),
            cycle: None,
            _tasks: Vec::new(),
        };
        browser.adblock_on.set(browser.settings.block_ads);
        browser._subscriptions = browser.subscribe_omnibox(window, cx);
        browser.apply_theme(window);
        browser.launched_engine = browser.settings.engine;
        if browser.settings.startup == Startup::StartPage {
            browser.start_fresh();
        }
        browser.open_startup_urls();
        browser.ensure_a_tab();
        browser.start_profile();
        browser.load_filter_lists(false, cx);
        browser._tasks = vec![
            spawn_pump(rx, cx),
            spawn_ticker(cx),
            spawn_incoming(incoming, cx),
        ];
        Ok(browser)
    }

    // ---- queries -------------------------------------------------------

    pub(crate) fn workspace(&self) -> Option<&Workspace> {
        self.session.active()
    }

    pub(crate) fn active_id(&self) -> Option<TabId> {
        self.workspace()?.active
    }

    pub(crate) fn active_tab(&self) -> Option<&SavedTab> {
        self.workspace()?.active_tab()
    }

    pub(crate) fn is_loading(&self, id: TabId) -> bool {
        self.live.get(&id).is_some_and(|t| t.loading)
    }

    pub(crate) fn is_playing(&self, id: TabId) -> bool {
        self.live.get(&id).is_some_and(|t| t.playing_audio)
    }

    pub(crate) fn blocked_count(&self) -> u32 {
        let Some(id) = self.active_id() else {
            return 0;
        };
        self.blocked.borrow().get(&id).copied().unwrap_or(0)
    }

    pub(crate) fn spring_params(&self) -> SpringParams {
        self.settings.motion.apply(self.theme.motion)
    }

    pub(crate) fn active_view_ref(&self) -> Option<&TabView> {
        self.active_view()
    }

    fn active_view(&self) -> Option<&TabView> {
        self.view(self.active_id()?)
    }

    pub(crate) fn view(&self, id: TabId) -> Option<&TabView> {
        self.live.get(&id)?.view.as_ref()
    }

    // ---- theme ---------------------------------------------------------

    pub(crate) fn apply_theme(&mut self, window: &mut Window) {
        // A theme removed since it was chosen falls back to Safari Glass.
        let id = self
            .settings
            .theme
            .clone()
            .unwrap_or_else(presets::default_id);
        let tokens = self
            .themes
            .get(&id)
            .cloned()
            .unwrap_or_else(presets::safari_glass_dark);
        let high_contrast = platform::high_contrast();
        let solid = self.settings.reduced_glass || high_contrast;
        self.theme = if solid { tokens.solid() } else { tokens };
        self.colors = if high_contrast {
            ChromeColors::high_contrast(platform::sys_color)
        } else {
            ChromeColors::from_theme(&self.theme)
        };
        self.motion.set_params(self.spring_params());
        self.springs.borrow_mut().set_params(self.spring_params());
        window.set_background_appearance(style::backdrop(&self.theme));
    }

    // ---- engine plumbing -----------------------------------------------

    fn start_profile(&mut self) {
        let Some(workspace) = self.workspace().map(|w| w.id) else {
            return;
        };
        let options = ProfileOptions {
            user_data_dir: self.paths.webview_data(workspace),
            language: None,
            extra_args: self
                .launched_engine
                .browser_args()
                .into_iter()
                .map(str::to_owned)
                .collect(),
        };
        let (slot, wake) = (self.profile.clone(), self.wake.clone());
        let (failures, starting) = (self.profile_failures.clone(), self.profile_starting.clone());
        starting.set(true);
        let started = Profile::create(&options, move |profile| {
            starting.set(false);
            match profile {
                Ok(profile) => {
                    failures.set(0);
                    *slot.borrow_mut() = Some(profile);
                }
                Err(e) => {
                    // Right after a restart the old browser process can still
                    // hold the data folder; the ticker tries again shortly.
                    failures.set(failures.get() + 1);
                    eprintln!("WebView2 failed to start: {e}");
                }
            }
            let _ = wake.unbounded_send(());
        });
        if let Err(e) = started {
            self.profile_starting.set(false);
            self.profile_failures.set(self.profile_failures.get() + 1);
            eprintln!("WebView2 failed to start: {e}");
        }
    }

    /// Runs after any engine callback: adopt new views, handle events,
    /// make sure the visible tab has a renderer.
    pub(crate) fn pump(&mut self, cx: &mut Context<Self>) {
        self.adopt_arrivals();
        self.apply_extension_results();
        self.apply_script_results();
        self.sync_extensions();
        self.adopt_preview();
        for event in self.queue.drain() {
            self.handle_event(event, cx);
        }
        self.ensure_visible_live();
        cx.notify();
    }

    fn adopt_arrivals(&mut self) {
        let arrivals: Vec<TabView> = self.arrivals.borrow_mut().drain(..).collect();
        for view in arrivals {
            let id = view.id();
            let Some(saved) = self.saved(id).cloned() else {
                continue; // closed while its controller was being created
            };
            restore_view(&view, &saved);
            if let Some(live) = self.live.get_mut(&id) {
                live.view = Some(view);
                live.visible = false;
            }
        }
        self.sync_views();
    }

    fn ensure_visible_live(&mut self) {
        for id in [self.active_id(), self.split].into_iter().flatten() {
            self.ensure_live(id);
        }
    }

    fn ensure_live(&mut self, id: TabId) {
        if self.live.contains_key(&id) {
            return;
        }
        // A fresh tab shows the native start page; it gets a renderer only
        // once it navigates somewhere.
        if self.saved(id).is_some_and(|t| t.url == NEW_TAB_URL) {
            return;
        }
        let Some(profile) = self.profile.borrow().clone() else {
            return; // retried on the next pump, once the profile is ready
        };
        let arrivals = self.arrivals.clone();
        let created = TabView::create(
            &profile,
            self.hwnd,
            id,
            self.hooks_for(id),
            self.queue.clone(),
            move |view| match view {
                Ok(view) => arrivals.borrow_mut().push(view),
                Err(e) => eprintln!("tab {id:?} failed: {e}"),
            },
        );
        match created {
            Ok(()) => {
                self.live.insert(id, LiveTab::pending());
            }
            Err(e) => eprintln!("tab {id:?} failed to start: {e}"),
        }
    }

    fn hooks_for(&self, id: TabId) -> TabHooks {
        let (blocker, counts) = (self.blocker.clone(), self.blocked.clone());
        let (enabled, exposure) = (self.adblock_on.clone(), self.exposure.clone());
        let filter = move |url: &str, page: &str, kind| {
            let block = enabled.get() && blocker.borrow().should_block(url, page, kind);
            if block {
                *counts.borrow_mut().entry(id).or_default() += 1;
            }
            exposure
                .borrow_mut()
                .entry(id)
                .or_default()
                .record(url, page, block);
            block
        };
        TabHooks {
            request_filter: Some(Rc::new(filter)),
            strip_tracking: self.settings.strip_tracking,
            login_bridge: true,
            user_scripts: self.scripts.clone(),
            page: self.page_settings(),
            privacy_probe: true,
            permissions: Some(Rc::new({
                let perms = self.permissions.clone();
                move |origin: &str, kind| perms.get(origin, kind)
            })),
        }
    }

    fn page_settings(&self) -> PageSettings {
        let s = &self.settings;
        PageSettings {
            block_popups: s.block_popups,
            tracking: s.tracking_prevention,
            downloads: DownloadTarget {
                folder: s.downloads.folder.clone(),
                ask_where: s.downloads.ask_where,
            },
        }
    }

    /// The browser process keeps the switches it started with.
    pub(crate) fn restart_needed(&self) -> bool {
        self.settings.engine != self.launched_engine
    }

    /// Relaunches with the same tabs. gpui waits for this process to exit
    /// before starting the new one, so the single-instance pipe is free.
    pub(crate) fn restart(&mut self, cx: &mut Context<Self>) {
        self.flush();
        self.live.clear();
        cx.restart();
    }

    /// "Start page" startup: last session's tabs move to the closed list,
    /// where Ctrl+Shift+T can still bring them back.
    fn start_fresh(&mut self) {
        for w in &mut self.session.workspaces {
            let ids: Vec<TabId> = w.tabs.iter().map(|t| t.id).collect();
            for id in ids.into_iter().rev() {
                w.close_tab(id);
            }
        }
        self.debounce.mark_dirty(Instant::now());
    }

    pub(crate) fn clear_site_data(&mut self, cx: &mut Context<Self>) {
        let Some(view) = self.live.values().find_map(|l| l.view.as_ref()) else {
            self.site_data.set(SiteData::NeedsPage);
            return cx.notify();
        };
        let (state, wake) = (self.site_data.clone(), self.wake.clone());
        let started = view.clear_site_data(move |ok| {
            state.set(if ok {
                SiteData::Cleared
            } else {
                SiteData::Failed
            });
            let _ = wake.unbounded_send(());
        });
        self.site_data.set(match started {
            Ok(()) => SiteData::Clearing,
            Err(_) => SiteData::Failed,
        });
        cx.notify();
    }

    pub(crate) fn site_data_status(&self) -> &'static str {
        match self.site_data.get() {
            SiteData::Idle => "Signs you out of sites in this workspace and empties the cache.",
            SiteData::Clearing => "Clearing…",
            SiteData::Cleared => "Cleared. Sites in this workspace have forgotten you.",
            SiteData::Failed => "Could not clear. This needs WebView2 runtime 1.0.1245 or newer.",
            SiteData::NeedsPage => "Open any web page in this workspace first, then clear.",
        }
    }

    pub(crate) fn open_blocked_popup(&mut self) {
        if let Some((parent, url)) = self.blocked_popup.take() {
            self.open_tab(&url, Some(parent), true);
        }
    }

    fn handle_event(&mut self, event: EngineEvent, cx: &mut Context<Self>) {
        let id = event.tab;
        match event.event {
            TabEvent::NavigationStarting { .. } => self.on_navigation_start(id),
            TabEvent::NavigationCompleted { success } => self.on_navigation_done(id, success),
            TabEvent::UrlChanged { url } => self.update_saved(id, |t| t.url = url),
            TabEvent::TitleChanged { title } => self.on_title(id, title),
            TabEvent::AudioChanged { playing } => self.set_live(id, |t| t.playing_audio = playing),
            TabEvent::NewWindowRequested { url } => self.open_tab(&url, Some(id), true),
            TabEvent::PopupBlocked { url } => self.blocked_popup = Some((id, url)),
            TabEvent::DownloadSaveAs {
                download,
                suggested,
            } => self.ask_save_path(id, download, suggested, cx),
            TabEvent::Suspended { ok } => self.set_live(id, |t| {
                t.residency = if ok {
                    Residency::Suspended
                } else {
                    Residency::Live
                }
            }),
            TabEvent::ProcessFailed => self.discard(id),
            TabEvent::Shortcut(shortcut) => self.run_shortcut(shortcut, None, cx),
            TabEvent::CreateFailed(error) => {
                eprintln!("tab {id:?}: {error}");
                self.live.remove(&id);
            }
            TabEvent::LoginFormSeen { origin } => self.on_login_form(id, origin),
            TabEvent::LoginSubmitted { origin, credential } => {
                self.on_login_submitted(origin, credential)
            }
            TabEvent::DownloadStarted {
                download,
                file,
                total,
            } => self.downloads.started(id, download, file, total),
            TabEvent::DownloadProgress {
                download,
                received,
                total,
            } => self.downloads.progress(download, received, total),
            TabEvent::DownloadFinished { download, ok } => {
                self.on_download_finished(download, ok, cx)
            }
            TabEvent::FaviconChanged { png } => self.on_favicon(id, png),
            TabEvent::FullscreenChanged { on } => self.on_page_fullscreen(id, on, cx),
            TabEvent::PageFocused => self.end_omnibox_editing(cx),
            TabEvent::PermissionRequested {
                request,
                origin,
                kind,
            } => self.permission_prompts.push(PermissionPrompt {
                tab: id,
                request,
                origin,
                kind,
            }),
            TabEvent::FindResult { active, count } => {
                if Some(id) == self.active_id() {
                    self.find_result = (active, count);
                }
            }
            TabEvent::CookiesImported { count } => self.on_cookies_imported(id, count, cx),
            TabEvent::WebRtcUsed => {
                self.exposure.borrow_mut().entry(id).or_default().webrtc = true;
            }
            TabEvent::Ready => self.flush_pending_cookies(id, cx),
        }
    }

    fn on_navigation_start(&mut self, id: TabId) {
        self.drop_permission_prompts(id);
        self.blocked.borrow_mut().remove(&id);
        self.exposure.borrow_mut().remove(&id);
        self.ip_alerted.retain(|t| *t != id);
        if self.ip_alert.is_some_and(|(t, _)| t == id) {
            self.ip_alert = None;
        }
        if self
            .blocked_popup
            .as_ref()
            .is_some_and(|(tab, _)| *tab == id)
        {
            self.blocked_popup = None;
        }
        self.set_live(id, |t| t.loading = true);
    }

    fn on_navigation_done(&mut self, id: TabId, success: bool) {
        self.set_live(id, |t| t.loading = false);
        let Some(tab) = self.saved(id) else {
            return;
        };
        if success && tab.url.starts_with("http") {
            let _ = self.history.record_visit(&tab.url, &tab.title, unix_now());
        }
    }

    fn on_title(&mut self, id: TabId, title: String) {
        if let Some(tab) = self.saved(id) {
            let _ = self.history.set_title(&tab.url, &title);
        }
        self.update_saved(id, |t| t.title = title);
    }

    // ---- tab state -----------------------------------------------------

    pub(crate) fn saved_tab(&self, id: TabId) -> Option<&SavedTab> {
        self.saved(id)
    }

    fn saved(&self, id: TabId) -> Option<&SavedTab> {
        self.workspace()?.tab(id)
    }

    fn workspace_mut(&mut self) -> Option<&mut Workspace> {
        let id = self.session.active_workspace;
        self.session.workspace_mut(id).ok()
    }

    fn update_saved(&mut self, id: TabId, change: impl FnOnce(&mut SavedTab)) {
        if let Some(tab) = self.workspace_mut().and_then(|w| w.tab_mut(id)) {
            change(tab);
            self.debounce.mark_dirty(Instant::now());
        }
    }

    fn set_live(&mut self, id: TabId, change: impl FnOnce(&mut LiveTab)) {
        if let Some(live) = self.live.get_mut(&id) {
            change(live);
        }
    }

    /// Another launch's URLs open as tabs here, and the window comes forward
    /// (the forwarding process granted us the foreground).
    fn open_forwarded(&mut self, args: Vec<String>, cx: &mut Context<Self>) {
        let template = self.settings.search_template.clone();
        for arg in args {
            if let Some(target) = OmniboxTarget::interpret(&arg) {
                self.open_tab(&target.into_url(&template), None, true);
            }
        }
        platform::bring_to_front(self.hwnd);
        cx.notify();
    }

    /// Loads the compiled lists, then downloads stale ones (all of them with
    /// `force`) and recompiles if anything changed. Everything heavy runs on
    /// a background thread; tabs pick up the new blocker on their next request.
    pub(crate) fn load_filter_lists(&self, force: bool, cx: &mut Context<Self>) {
        let (dir, cache) = (self.paths.filters(), self.paths.adblock_cache());
        let slot = self.blocker.clone();
        cx.spawn(async move |_, cx| {
            let compile = |cx: &gpui::AsyncApp| {
                let (dir, cache) = (dir.clone(), cache.clone());
                cx.background_spawn(async move { compile_lists(dir, cache) })
            };
            if let Some(blocker) = compile(cx).await {
                *slot.borrow_mut() = Arc::new(blocker);
            }
            let refresh_dir = dir.clone();
            let (changed, errors) = cx
                .background_spawn(async move {
                    lists::refresh(&refresh_dir, lists::DEFAULT_LISTS, force)
                })
                .await;
            for e in errors {
                eprintln!("filter list update failed: {e}");
            }
            if changed && let Some(blocker) = compile(cx).await {
                *slot.borrow_mut() = Arc::new(blocker);
            }
        })
        .detach();
    }

    /// The icon for a URL's host: memory first, then the disk cache. Misses
    /// are remembered too, so rendering never touches the disk twice.
    pub(crate) fn icon_for(&self, url: &str) -> Option<Arc<gpui::Image>> {
        let host = host_of(url)?;
        if let Some(icon) = self.icons.borrow().get(&host) {
            return icon.clone();
        }
        let icon = self.favicons.get(&host).map(png_image);
        self.icons.borrow_mut().insert(host, icon.clone());
        icon
    }

    fn on_favicon(&mut self, id: TabId, png: Vec<u8>) {
        let Some(host) = self.saved(id).and_then(|t| host_of(&t.url)) else {
            return;
        };
        if let Err(e) = self.favicons.put(&host, &png) {
            eprintln!("favicon cache: {e}");
        }
        let icon = (!png.is_empty()).then(|| png_image(png));
        self.icons.borrow_mut().insert(host, icon);
    }

    pub(crate) fn other_workspaces(&self) -> Vec<(browser_core::WorkspaceId, String)> {
        let active = self.session.active_workspace;
        self.session
            .workspaces
            .iter()
            .filter(|w| w.id != active)
            .map(|w| (w.id, w.name.clone()))
            .collect()
    }

    pub(crate) fn has_many_workspaces(&self) -> bool {
        self.session.workspaces.len() > 1
    }

    pub(crate) fn new_workspace(&mut self, name: String) {
        let name = if name.is_empty() {
            format!("Workspace {}", self.session.workspaces.len() + 1)
        } else {
            name
        };
        let id = self.session.add_workspace(name);
        self.switch_workspace(id);
    }

    /// Each workspace has its own WebView2 data folder, so switching closes
    /// this workspace's pages and starts the other workspace's profile. Its
    /// tabs come back lazily, like a session restore.
    pub(crate) fn switch_workspace(&mut self, id: browser_core::WorkspaceId) {
        if id == self.session.active_workspace || self.session.switch_workspace(id).is_err() {
            return;
        }
        self.live.clear();
        self.split = None;
        self.suggestions.clear();
        *self.profile.borrow_mut() = None;
        self.extensions_synced = false;
        self.ensure_a_tab();
        self.start_profile();
        self.debounce.mark_dirty(Instant::now());
    }

    pub(crate) fn rename_workspace(&mut self, name: String) {
        if let Some(w) = self.workspace_mut() {
            w.name = name;
        }
        self.debounce.mark_dirty(Instant::now());
    }

    /// `umbrella.exe <url>...` opens each argument as a tab.
    fn open_startup_urls(&mut self) {
        let template = self.settings.search_template.clone();
        for arg in std::env::args().skip(1) {
            if let Some(target) = OmniboxTarget::interpret(&arg) {
                self.open_tab(&target.into_url(&template), None, true);
            }
        }
    }

    fn ensure_a_tab(&mut self) {
        if self.workspace().is_some_and(|w| w.tabs.is_empty()) {
            self.open_tab(NEW_TAB_URL, None, true);
        }
    }

    pub(crate) fn open_tab(&mut self, url: &str, parent: Option<TabId>, activate: bool) {
        if activate {
            // A new foreground tab takes over from Settings.
            self.settings_open = false;
        }
        let ws = self.session.active_workspace;
        let previous = self.active_id();
        let Ok(id) = self.session.open_tab(ws, url, parent) else {
            return;
        };
        let zoom = self.settings.default_zoom;
        self.update_saved(id, |t| t.set_zoom(zoom));
        if let (false, Some(prev), Some(w)) = (activate, previous, self.workspace_mut()) {
            w.activate(prev);
        }
        self.after_tab_change(id);
    }

    pub(crate) fn close_tab(&mut self, id: TabId) {
        let closed = self.workspace_mut().and_then(|w| w.close_tab(id));
        if closed.is_none() {
            return;
        }
        self.permission_prompts.retain(|p| p.tab != id);
        self.live.remove(&id); // dropping the view closes the controller
        if self.split == Some(id) {
            self.split = None;
        }
        self.ensure_a_tab();
        self.after_tab_change(id);
    }

    pub(crate) fn reopen_last(&mut self) {
        if let Some(id) = self.workspace_mut().and_then(|w| w.reopen_closed()) {
            self.after_tab_change(id);
        }
    }

    /// The tab the strip shows as current: Settings while it is open.
    pub(crate) fn strip_active(&self) -> Option<TabId> {
        if self.settings_open {
            Some(SETTINGS_TAB)
        } else {
            self.active_id()
        }
    }

    pub(crate) fn activate_tab(&mut self, id: TabId) {
        // Picking any page tab leaves Settings, like switching away from it.
        self.settings_open = false;
        if self.split == Some(id) {
            self.split = self.active_id();
        }
        if let Some(w) = self.workspace_mut() {
            w.activate(id);
        }
        self.after_tab_change(id);
    }

    /// Ctrl+Tab steps through MRU order without reordering it; the choice
    /// commits (and moves to the front) when Ctrl is released.
    fn cycle_tabs(&mut self, forward: bool) {
        self.settings_open = false;
        let Some(w) = self.workspace() else {
            return;
        };
        let len = w.mru.len().max(1);
        let step = self
            .cycle
            .map_or(1, |s| if forward { s + 1 } else { s + len - 1 })
            % len;
        let Some(target) = w.mru.back(step) else {
            return;
        };
        self.cycle = Some(step);
        if let Some(w) = self.workspace_mut() {
            w.active = Some(target);
        }
        self.after_tab_change(target);
    }

    pub(crate) fn commit_cycle(&mut self) {
        if self.cycle.take().is_none() {
            return;
        }
        if let Some(id) = self.active_id() {
            self.activate_tab(id);
        }
    }

    fn after_tab_change(&mut self, _id: TabId) {
        // Matches belong to the page they were found on.
        if self.find_open {
            self.find_open = false;
            self.find_result = (0, 0);
        }
        self.debounce.mark_dirty(Instant::now());
        self.ensure_visible_live();
        self.sync_views();
    }

    fn discard(&mut self, id: TabId) {
        self.live.remove(&id);
        self.ensure_visible_live();
    }

    // ---- views ---------------------------------------------------------

    /// Shows the active (and split) tab's view, hides the rest, and keeps
    /// bounds current.
    pub(crate) fn sync_views(&mut self) {
        let active = self.active_id();
        let split = self.split;
        // The suggestion panel takes the page area; pages cannot be drawn over.
        // Suggestions and settings take the page area; pages cannot be drawn over.
        let sheet = self.settings_open || self.downloads_open || self.page_info_open;
        let panel = self.suggestions_open() || sheet || self.on_start_page();
        if !self.omnibox_editing && !sheet {
            self.preview = None;
        }
        let now = Instant::now();
        for (id, live) in self.live.iter_mut() {
            let visible = !panel && (Some(*id) == active || Some(*id) == split);
            if live.view.is_some() && visible != live.visible {
                show_view(live, visible, now);
            }
        }
        self.apply_pane_bounds();
    }

    pub(crate) fn set_content_bounds(&mut self, rect: LogicalRect, scale: f32) {
        if self.content_rect == Some((rect, scale)) {
            return;
        }
        self.content_rect = Some((rect, scale));
        self.apply_pane_bounds();
    }

    fn apply_pane_bounds(&self) {
        let Some((rect, scale)) = self.content_rect else {
            return;
        };
        let (main, side) = split_rects(rect, self.split.is_some(), self.split_ratio);
        let active = self.active_id();
        for (id, live) in &self.live {
            let pane = if Some(*id) == active {
                Some(main)
            } else if Some(*id) == self.split {
                side
            } else {
                None
            };
            if let (Some(pane), Some(view)) = (pane, &live.view) {
                let _ = view.set_bounds(pane.to_physical(scale));
            }
        }
    }

    /// Where the split gutter starts, in window x, while split view is on.
    pub(crate) fn split_gutter_x(&self) -> Option<f32> {
        self.split?;
        let (rect, _) = self.content_rect?;
        let (main, _) = split_rects(rect, true, self.split_ratio);
        Some(main.x + main.width)
    }

    /// Follows the pointer during a gutter drag; panes stay at least 20 %.
    pub(crate) fn drag_split_to(&mut self, pointer_x: f32) {
        let Some((rect, _)) = self.content_rect else {
            return;
        };
        let usable = (rect.width - SPLIT_GUTTER).max(1.0);
        self.split_ratio = ((pointer_x - rect.x - SPLIT_GUTTER / 2.0) / usable).clamp(0.2, 0.8);
        self.apply_pane_bounds();
    }

    fn on_page_fullscreen(&mut self, id: TabId, on: bool, cx: &mut Context<Self>) {
        if Some(id) != self.active_id() || on == self.page_fullscreen {
            return;
        }
        self.page_fullscreen = on;
        self.defer_in_window(cx, move |_, window, _| {
            if window.is_fullscreen() != on {
                window.toggle_fullscreen();
            }
        });
    }

    pub(crate) fn filter_rule_count(&self) -> usize {
        self.blocker.borrow().rule_count()
    }

    pub(crate) fn toggle_split(&mut self) {
        if self.split.take().is_some() {
            self.sync_views();
            return;
        }
        let active = self.active_id();
        let mut partner = self
            .workspace()
            .and_then(|w| w.mru.iter().find(|id| Some(*id) != active));
        if partner.is_none() {
            self.open_tab(NEW_TAB_URL, None, false);
            partner = self.workspace().and_then(|w| w.tabs.last()).map(|t| t.id);
        }
        self.split = partner.filter(|p| Some(*p) != active);
        self.ensure_visible_live();
        self.sync_views();
    }

    pub(crate) fn focus_page(&self) {
        if let Some(view) = self.active_view() {
            let _ = view.focus();
        }
    }

    // ---- navigation ----------------------------------------------------

    pub(crate) fn navigate_active(&mut self, input: &str) {
        let Some(target) = OmniboxTarget::interpret(input) else {
            return;
        };
        let url = target.into_url(&self.settings.search_template);
        let Some(id) = self.active_id() else {
            return;
        };
        self.update_saved(id, |t| t.url = url.clone());
        match self.active_view() {
            Some(view) => {
                let _ = view.navigate(&url);
                self.focus_page();
            }
            None => self.ensure_live(id),
        }
    }

    pub(crate) fn with_active(&self, act: impl FnOnce(&TabView)) {
        self.with_active_view(act);
    }

    fn with_active_view(&self, act: impl FnOnce(&TabView)) {
        if let Some(view) = self.active_view() {
            act(view);
        }
    }

    fn zoom(&mut self, delta: Option<f32>) {
        let Some(id) = self.active_id() else {
            return;
        };
        let (mut zoom, reset) = (DEFAULT_ZOOM, self.settings.default_zoom);
        self.update_saved(id, |t| {
            t.set_zoom(delta.map_or(reset, |d| t.zoom + d));
            zoom = t.zoom;
        });
        self.with_active_view(|v| {
            let _ = v.set_zoom(f64::from(zoom));
        });
    }

    pub(crate) fn toggle_mute(&mut self, id: TabId) {
        let mut muted = false;
        self.update_saved(id, |t| {
            t.muted = !t.muted;
            muted = t.muted;
        });
        if let Some(view) = self.live.get(&id).and_then(|l| l.view.as_ref()) {
            let _ = view.set_muted(muted);
        }
    }

    pub(crate) fn toggle_bookmark(&mut self) {
        let Some(tab) = self.active_tab() else {
            return;
        };
        let _ = self.bookmarks.toggle(&tab.url, &tab.title, unix_now());
    }

    pub(crate) fn is_bookmarked(&self) -> bool {
        self.active_tab()
            .is_some_and(|t| self.bookmarks.contains(&t.url).unwrap_or(false))
    }

    pub(crate) fn go_back(&self) {
        self.with_active_view(|v| {
            let _ = v.go_back();
        });
    }

    pub(crate) fn go_forward(&self) {
        self.with_active_view(|v| {
            let _ = v.go_forward();
        });
    }

    pub(crate) fn reload_or_stop(&self) {
        let loading = self.active_id().is_some_and(|id| self.is_loading(id));
        self.with_active_view(|v| {
            let _ = if loading { v.stop() } else { v.reload() };
        });
    }

    // ---- shortcuts -----------------------------------------------------

    pub(crate) fn run_shortcut(
        &mut self,
        shortcut: Shortcut,
        window: Option<&mut Window>,
        cx: &mut Context<Self>,
    ) {
        match shortcut {
            Shortcut::Find => match window {
                Some(window) => self.open_find(window, cx),
                None => self.defer_in_window(cx, |b, window, cx| b.open_find(window, cx)),
            },
            Shortcut::NewTab => {
                self.open_tab(NEW_TAB_URL, None, true);
                self.request_omnibox_focus(window, cx);
            }
            Shortcut::CloseTab => {
                if self.settings_open {
                    self.close_settings(cx);
                } else if let Some(id) = self.active_id() {
                    self.close_tab(id);
                }
            }
            Shortcut::ReopenTab => self.reopen_last(),
            Shortcut::NextTab => self.cycle_tabs(true),
            Shortcut::PreviousTab => self.cycle_tabs(false),
            Shortcut::CtrlReleased => self.commit_cycle(),
            Shortcut::FocusOmnibox => self.request_omnibox_focus(window, cx),
            Shortcut::ZoomIn => self.zoom(Some(ZOOM_STEP)),
            Shortcut::ZoomOut => self.zoom(Some(-ZOOM_STEP)),
            Shortcut::ZoomReset => self.zoom(None),
            Shortcut::ToggleBookmark => self.toggle_bookmark(),
            Shortcut::Back => self.go_back(),
            Shortcut::Forward => self.go_forward(),
            Shortcut::Settings => self.open_settings(cx),
            Shortcut::History => {
                self.defer_in_window(cx, |b, window, cx| b.open_history(window, cx))
            }
            Shortcut::MoveTabLeft => self.move_active_tab(-1),
            Shortcut::MoveTabRight => self.move_active_tab(1),
            Shortcut::CommandPalette => {
                self.defer_in_window(cx, |b, window, cx| b.open_palette(window, cx))
            }
        }
        cx.notify();
    }

    /// Shortcuts from a page arrive without a `Window`; defer to the next frame.
    fn request_omnibox_focus(&mut self, window: Option<&mut Window>, cx: &mut Context<Self>) {
        match window {
            Some(window) => self.focus_omnibox(window, cx),
            None => self.defer_in_window(cx, |b, window, cx| b.focus_omnibox(window, cx)),
        }
    }

    /// Runs `act` with the main window once the current update has finished,
    /// for work that starts outside a window (engine events, popup clicks).
    pub(crate) fn defer_in_window(
        &self,
        cx: &mut Context<Self>,
        act: impl FnOnce(&mut Browser, &mut Window, &mut Context<Browser>) + 'static,
    ) {
        let (entity, handle) = (cx.entity(), self.window);
        cx.defer(move |cx| {
            let _ = handle.update(cx, |_, window, cx| {
                entity.update(cx, |b, cx| act(b, window, cx))
            });
        });
    }

    fn subscribe_omnibox(
        &mut self,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) -> Vec<Subscription> {
        let handle = self.omnibox.read(cx).focus_handle(cx);
        vec![
            cx.subscribe(&self.omnibox, |this, _, _: &TextChanged, cx| {
                this.refresh_suggestions(cx)
            }),
            cx.subscribe(&self.find_field, |this, _, _: &TextChanged, cx| {
                this.run_find(cx)
            }),
            cx.subscribe(&self.theme_name_field, |this, _, _: &TextChanged, cx| {
                this.rename_theme(cx)
            }),
            cx.subscribe(&self.theme_hex_field, |this, _, _: &TextChanged, cx| {
                this.hex_typed(cx)
            }),
            cx.on_blur(&handle, window, |this, window, cx| {
                this.on_omnibox_blur(window, cx)
            }),
            // While a page has keyboard focus GPUI sees its own window as
            // inactive (the page's input window lives in the WebView2 process).
            // A click on the omnibox focuses it before the window reactivates,
            // and that reactivation arrives as a blur. Hand focus back here.
            cx.observe_window_activation(window, |this, window, cx| {
                if window.is_window_active() && this.omnibox_editing {
                    this.refocus_omnibox(window, cx);
                }
            }),
            // Light/dark and high-contrast switches arrive as appearance changes.
            cx.observe_window_appearance(window, |this, window, cx| {
                this.apply_theme(window);
                cx.notify();
            }),
        ]
    }

    pub(crate) fn update_settings(
        &mut self,
        cx: &mut Context<Self>,
        change: impl FnOnce(&mut Settings),
    ) {
        change(&mut self.settings);
        self.adblock_on.set(self.settings.block_ads);
        let page = self.page_settings();
        for view in self.live.values().filter_map(|l| l.view.as_ref()) {
            view.apply_page_settings(&page);
        }
        if let Err(e) = self.settings.save(&self.kv) {
            eprintln!("settings save failed: {e}");
        }
        self.defer_in_window(cx, |b, window, _| b.apply_theme(window));
    }

    fn refocus_omnibox(&mut self, window: &mut Window, cx: &mut Context<Self>) {
        // The blur already reset the selection, so place it again even when
        // the handle still counts as focused.
        let handle = self.omnibox.read(cx).focus_handle(cx);
        window.focus(&handle, cx);
        self.place_omnibox_caret(cx);
        cx.notify();
    }

    fn place_omnibox_caret(&self, cx: &mut Context<Self>) {
        let select_all = self.omnibox_select_all;
        self.omnibox.update(cx, |field, cx| {
            if select_all {
                field.select_document(cx);
            } else {
                field.move_to(usize::MAX, cx);
            }
        });
    }

    /// A click on the address: caret at the end, nothing selected.
    pub(crate) fn click_omnibox(&mut self, window: &mut Window, cx: &mut Context<Self>) {
        self.omnibox_select_all = false;
        self.open_omnibox(window, cx);
    }

    /// Ctrl+L, a new tab, the palette: everything selected, ready to replace.
    pub(crate) fn focus_omnibox(&mut self, window: &mut Window, cx: &mut Context<Self>) {
        self.omnibox_select_all = true;
        self.open_omnibox(window, cx);
    }

    fn open_omnibox(&mut self, window: &mut Window, cx: &mut Context<Self>) {
        // Typing an address on the Settings tab leaves it for the page tab.
        if self.settings_open {
            self.settings_open = false;
            self.sync_views();
        }
        platform::take_focus(self.hwnd);
        let url = self.active_tab().map(|t| t.url.clone()).unwrap_or_default();
        let text = if url == NEW_TAB_URL {
            String::new()
        } else {
            url
        };
        self.omnibox
            .update(cx, |field, cx| field.emplace(&text, cx));
        self.place_omnibox_caret(cx);
        self.omnibox_editing = true;
        self.capture_page_preview();
        let handle = self.omnibox.read(cx).focus_handle(cx);
        window.focus(&handle, cx);
        cx.notify();
    }

    // ---- background work -----------------------------------------------

    /// Starts a memory reading for every live tab; results land in
    /// `tab_memory` and wake the UI. Cheap enough to run every few seconds
    /// while a hover card is up.
    pub(crate) fn refresh_tab_memory(&mut self) {
        let Some(profile) = self.profile.borrow().clone() else {
            return;
        };
        self.memory_checked = Some(Instant::now());
        let frames: Vec<(TabId, u32)> = self
            .live
            .iter()
            .filter_map(|(id, l)| Some((*id, l.view.as_ref()?.main_frame_id()?)))
            .collect();
        let (slot, wake) = (self.tab_memory.clone(), self.wake.clone());
        let started = profile.renderer_memory(move |renderers| {
            let mut readings = HashMap::new();
            for (tab, frame) in frames {
                let Some(r) = renderers.iter().find(|r| r.main_frames.contains(&frame)) else {
                    continue;
                };
                let shared = r.main_frames.len();
                readings.insert(
                    tab,
                    MemoryReading {
                        bytes: r.bytes,
                        shared,
                    },
                );
            }
            *slot.borrow_mut() = readings;
            let _ = wake.unbounded_send(());
        });
        if let Err(e) = started {
            eprintln!("memory reading unavailable: {e}");
        }
    }

    /// Raises the corner notice once per page when the active tab's page
    /// looked up the IP address; true while the notice needs repainting.
    pub(crate) fn check_ip_alert(&mut self) -> bool {
        if let Some((_, since)) = self.ip_alert {
            if since.elapsed() >= IP_ALERT_SHOWN + Duration::from_millis(600) {
                self.ip_alert = None;
            }
            return true;
        }
        let Some(id) = self.active_id() else {
            return false;
        };
        let flagged = self
            .exposure
            .borrow()
            .get(&id)
            .is_some_and(Exposure::looks_up_ip);
        if !self.settings.warn_ip_lookups || !flagged || self.ip_alerted.contains(&id) {
            return false;
        }
        self.ip_alerted.push(id);
        self.ip_alert = Some((id, Instant::now()));
        true
    }

    /// The hover card waits a beat so sweeping across tabs does not flash it.
    pub(crate) fn hover_card_due(&mut self) -> bool {
        let Some((_, since)) = self.hovered_tab else {
            return false;
        };
        let stale = self
            .memory_checked
            .is_none_or(|t| t.elapsed() >= MEMORY_REFRESH);
        if stale {
            self.refresh_tab_memory();
        }
        since.elapsed() >= HOVER_CARD_DELAY
    }

    /// The prompt shown for the active tab, if any.
    pub(crate) fn active_permission_prompt(&self) -> Option<&PermissionPrompt> {
        let active = self.active_id()?;
        self.permission_prompts.iter().find(|p| p.tab == active)
    }

    /// `Some(allow)` answers and remembers; `None` refuses this once.
    pub(crate) fn answer_permission(&mut self, request: u64, allow: Option<bool>) {
        let Some(i) = self
            .permission_prompts
            .iter()
            .position(|p| p.request == request)
        else {
            return;
        };
        let prompt = self.permission_prompts.remove(i);
        if let Some(allow) = allow
            && let Err(e) = self.permissions.set(&prompt.origin, prompt.kind, allow)
        {
            eprintln!("permission save failed: {e}");
        }
        if let Some(view) = self.view(prompt.tab) {
            let _ = view.resolve_permission(request, allow);
        }
    }

    /// A page that navigates away no longer needs its answer.
    fn drop_permission_prompts(&mut self, tab: TabId) {
        let requests: Vec<u64> = self
            .permission_prompts
            .iter()
            .filter(|p| p.tab == tab)
            .map(|p| p.request)
            .collect();
        for request in requests {
            self.answer_permission(request, None);
        }
    }

    /// Engine callbacks send on this to wake the UI.
    pub(crate) fn kv(&self) -> &KvStore {
        &self.kv
    }

    /// Any live page; profile-wide calls (extensions) go through one.
    pub(crate) fn any_view(&self) -> Option<&TabView> {
        self.live.values().find_map(|l| l.view.as_ref())
    }

    pub(crate) fn wake_handle(&self) -> mpsc::UnboundedSender<()> {
        self.wake.clone()
    }

    pub(crate) fn tab_state(&self, id: TabId) -> TabState {
        match self.live.get(&id) {
            None => TabState::NotLoaded,
            Some(l) if l.residency == Residency::Suspended => TabState::Asleep,
            Some(_) => TabState::Live,
        }
    }

    pub(crate) fn tick(&mut self) {
        let failures = self.profile_failures.get();
        let idle = self.profile.borrow().is_none() && !self.profile_starting.get();
        if idle && (1..=PROFILE_RETRIES).contains(&failures) {
            self.start_profile();
        }
        self.vault
            .lock_if_idle(self.settings.vault_idle_lock_minutes);
        if self.debounce.take_due(Instant::now()) {
            self.save();
        }
    }

    pub(crate) fn hibernate(&mut self) {
        let activity: Vec<TabActivity> = self
            .live
            .iter()
            .map(|(id, l)| TabActivity {
                id: *id,
                residency: l.residency,
                visible: l.visible,
                playing_audio: l.playing_audio,
                hidden_since: l.hidden_since,
            })
            .collect();
        for action in HibernatePolicy::default().plan(&activity, Instant::now()) {
            match action {
                HibernateAction::Suspend(id) => {
                    if let Some(view) = self.live.get(&id).and_then(|l| l.view.as_ref()) {
                        let _ = view.suspend();
                    }
                }
                HibernateAction::Discard(id) => {
                    self.live.remove(&id);
                }
            }
        }
    }

    pub(crate) fn save(&self) {
        if let Err(e) = self.store.save(&self.session) {
            eprintln!("session save failed: {e}");
        }
    }

    pub(crate) fn flush(&mut self) {
        if self.debounce.take_pending() {
            self.save();
        }
        let _ = self.settings.save(&self.kv);
    }
}

/// Progress of "Clear cookies and site data", shown in settings.
#[derive(Debug, Clone, Copy, Default, PartialEq, Eq)]
pub(crate) enum SiteData {
    #[default]
    Idle,
    Clearing,
    Cleared,
    Failed,
    NeedsPage,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub(crate) struct PermissionPrompt {
    pub tab: TabId,
    pub request: u64,
    pub origin: String,
    pub kind: PermissionKind,
}

/// Private working set of the renderer drawing a tab. `shared` counts the
/// tabs that renderer draws (same-site tabs can share one process).
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub(crate) struct MemoryReading {
    pub bytes: u64,
    pub shared: usize,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub(crate) enum TabState {
    NotLoaded,
    Asleep,
    Live,
}

/// How long the "looked up your IP" notice stays before it fades.
pub(crate) const IP_ALERT_SHOWN: Duration = Duration::from_secs(12);
const HOVER_CARD_DELAY: Duration = Duration::from_millis(450);
const MEMORY_REFRESH: Duration = Duration::from_secs(2);

/// How many times to retry starting WebView2 before giving up.
const PROFILE_RETRIES: u32 = 40;

/// A tab being dragged in the strip. `grab` is where the pointer caught the
/// tab, relative to its left edge.
#[derive(Debug, Clone, Copy)]
pub struct TabDrag {
    pub id: TabId,
    pub grab: f32,
    pub start: f32,
    pub pointer: f32,
    pub moved: bool,
}

/// Pointer travel before a press on a tab becomes a drag.
const DRAG_THRESHOLD: f32 = 4.0;

impl Browser {
    pub(crate) fn begin_tab_drag(&mut self, id: TabId, pointer: f32, tab_x: f32) {
        self.tab_drag = Some(TabDrag {
            id,
            grab: pointer - tab_x,
            start: pointer,
            pointer,
            moved: false,
        });
    }

    /// Follows the pointer and swaps the tab into the slot under it; the
    /// layout springs then glide the neighbours aside.
    pub(crate) fn drag_tab_to(&mut self, pointer: f32, strip_start: f32, gap: f32) {
        let Some(mut drag) = self.tab_drag else {
            return;
        };
        drag.pointer = pointer;
        drag.moved |= (pointer - drag.start).abs() > DRAG_THRESHOLD;
        self.tab_drag = Some(drag);
        if !drag.moved || self.tab_width <= 0.0 {
            return;
        }
        let center = pointer - drag.grab + self.tab_width / 2.0;
        let slot = ((center - strip_start) / (self.tab_width + gap))
            .floor()
            .max(0.0) as usize;
        let current = self.workspace().and_then(|w| w.position(drag.id));
        if current != Some(slot) {
            if let Some(w) = self.workspace_mut() {
                w.move_tab(drag.id, slot);
            }
            self.debounce.mark_dirty(Instant::now());
        }
    }

    pub(crate) fn end_tab_drag(&mut self) {
        self.tab_drag = None;
    }

    fn move_active_tab(&mut self, delta: isize) {
        let Some(id) = self.active_id() else {
            return;
        };
        let Some(index) = self.workspace().and_then(|w| w.position(id)) else {
            return;
        };
        let target = index.saturating_add_signed(delta);
        if let Some(w) = self.workspace_mut() {
            w.move_tab(id, target);
        }
        self.debounce.mark_dirty(Instant::now());
    }

    /// Takes a snapshot of the visible page so the suggestion panel can show
    /// it dimmed instead of an empty area while the live view is hidden.
    pub(crate) fn capture_page_preview(&mut self) {
        self.preview = None;
        let Some(view) = self.active_view() else {
            return;
        };
        let (slot, wake) = (self.preview_bytes.clone(), self.wake.clone());
        let started = view.capture_preview(move |bytes| {
            *slot.borrow_mut() = bytes;
            let _ = wake.unbounded_send(());
        });
        if let Err(e) = started {
            eprintln!("page preview failed: {e}");
        }
    }

    fn adopt_preview(&mut self) {
        let Some(bytes) = self.preview_bytes.borrow_mut().take() else {
            return;
        };
        if self.omnibox_editing {
            self.preview = Some(Arc::new(gpui::Image::from_bytes(
                gpui::ImageFormat::Png,
                bytes,
            )));
        }
    }
}

/// The main pane and, when split, the side pane with a gutter between.
/// `ratio` is the main pane's share of the width without the gutter.
fn split_rects(rect: LogicalRect, split: bool, ratio: f32) -> (LogicalRect, Option<LogicalRect>) {
    if !split {
        return (rect, None);
    }
    let usable = (rect.width - SPLIT_GUTTER).max(0.0);
    let main_width = (usable * ratio).round();
    let main = LogicalRect {
        width: main_width,
        ..rect
    };
    let side = LogicalRect {
        x: rect.x + main_width + SPLIT_GUTTER,
        width: usable - main_width,
        ..rect
    };
    (main, Some(side))
}

fn restore_view(view: &TabView, saved: &SavedTab) {
    let url = if saved.url.is_empty() {
        NEW_TAB_URL
    } else {
        &saved.url
    };
    let _ = view.navigate(url);
    if saved.zoom != DEFAULT_ZOOM {
        let _ = view.set_zoom(f64::from(saved.zoom));
    }
    if saved.muted {
        let _ = view.set_muted(true);
    }
}

fn show_view(live: &mut LiveTab, visible: bool, now: Instant) {
    live.visible = visible;
    live.hidden_since = (!visible).then_some(now);
    let Some(view) = &live.view else {
        return;
    };
    if visible && live.residency == Residency::Suspended {
        live.residency = Residency::Live;
        let _ = view.resume();
        return;
    }
    let _ = view.set_visible(visible);
}

fn compile_lists(dir: PathBuf, cache: PathBuf) -> Option<Blocker> {
    match Blocker::load_cached(&dir, &cache) {
        Ok(blocker) => Some(blocker),
        Err(e) => {
            eprintln!("filter lists unreadable: {e}");
            None
        }
    }
}

fn load_scripts(paths: &Paths) -> Vec<String> {
    match ScriptLibrary::load_dir(&paths.scripts()) {
        Ok(library) => {
            for (path, e) in &library.errors {
                eprintln!("userscript {} skipped: {e}", path.display());
            }
            library.bundles()
        }
        Err(e) => {
            eprintln!("scripts folder unreadable: {e}");
            Vec::new()
        }
    }
}

fn spawn_pump(mut rx: mpsc::UnboundedReceiver<()>, cx: &mut Context<Browser>) -> Task<()> {
    cx.spawn(async move |this, cx| {
        while rx.next().await.is_some() {
            if this.update(cx, |b, cx| b.pump(cx)).is_err() {
                break;
            }
        }
    })
}

/// Launches that found this window already running hand their URLs here.
fn spawn_incoming(mut incoming: Incoming, cx: &mut Context<Browser>) -> Task<()> {
    cx.spawn(async move |this, cx| {
        while let Some(args) = incoming.next().await {
            if this.update(cx, |b, cx| b.open_forwarded(args, cx)).is_err() {
                break;
            }
        }
    })
}

fn spawn_ticker(cx: &mut Context<Browser>) -> Task<()> {
    cx.spawn(async move |this, cx| {
        let mut since_hibernate = Duration::ZERO;
        loop {
            cx.background_executor().timer(SAVE_TICK).await;
            since_hibernate += SAVE_TICK;
            let run_hibernate = since_hibernate >= HIBERNATE_TICK;
            if run_hibernate {
                since_hibernate = Duration::ZERO;
            }
            let alive = this.update(cx, |b, cx| {
                b.tick();
                if b.hover_card_due() | b.check_ip_alert() | b.toast_visible() {
                    cx.notify();
                }
                if run_hibernate {
                    b.hibernate();
                }
            });
            if alive.is_err() {
                break;
            }
        }
    })
}

/// Host without a leading "www.", the key for icons.
pub fn host_of(url: &str) -> Option<String> {
    let parsed = url::Url::parse(url).ok()?;
    Some(
        parsed
            .host_str()?
            .trim_start_matches("www.")
            .to_ascii_lowercase(),
    )
}

fn png_image(png: Vec<u8>) -> Arc<gpui::Image> {
    Arc::new(gpui::Image::from_bytes(gpui::ImageFormat::Png, png))
}

pub fn unix_now() -> u64 {
    SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .map_or(0, |d| d.as_secs())
}
