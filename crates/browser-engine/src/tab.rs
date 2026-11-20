use std::cell::{Cell, RefCell};
use std::path::PathBuf;
use std::rc::Rc;

use browser_core::TabId;
use browser_core::settings::TrackingLevel;
use browser_core::vault::Origin;
use browser_net::{ResourceKind, strip_tracking};
use webview2_com::Microsoft::Web::WebView2::Win32::*;
use webview2_com::{
    AcceleratorKeyPressedEventHandler, AddScriptToExecuteOnDocumentCreatedCompletedHandler,
    CapturePreviewCompletedHandler, ClearBrowsingDataCompletedHandler,
    ContainsFullScreenElementChangedEventHandler, CreateCoreWebView2ControllerCompletedHandler,
    DocumentTitleChangedEventHandler, ExecuteScriptCompletedHandler, FaviconChangedEventHandler,
    FocusChangedEventHandler, GetFaviconCompletedHandler,
    IsDocumentPlayingAudioChangedEventHandler, NavigationCompletedEventHandler,
    NavigationStartingEventHandler, NewWindowRequestedEventHandler, ProcessFailedEventHandler,
    SourceChangedEventHandler, TrySuspendCompletedHandler, WebMessageReceivedEventHandler,
    WebResourceRequestedEventHandler, take_pwstr,
};
use windows::Win32::Foundation::HGLOBAL;
use windows::Win32::Foundation::HWND;
use windows::Win32::System::Com::StructuredStorage::{CreateStreamOnHGlobal, GetHGlobalFromStream};
use windows::Win32::System::Com::{IStream, STATFLAG_NONAME, STATSTG};
use windows::Win32::System::Memory::{GlobalLock, GlobalUnlock};
use windows::Win32::UI::Input::KeyboardAndMouse::{GetKeyState, VK_CONTROL, VK_MENU, VK_SHIFT};
use windows::core::{BOOL, HSTRING, Interface, PWSTR, w};
use zeroize::Zeroizing;

use crate::downloads::{self, DownloadId, DownloadTarget, Operations};
use crate::login::{self, LOGIN_BRIDGE_JS, LoginMessage};
use crate::shortcuts::{self, Modifiers};
use crate::{EngineError, EventQueue, PhysicalRect, Profile, TabEvent};

type WinResult<T> = windows::core::Result<T>;

/// Decides per request whether to block: `(url, page_url, kind) -> block`.
pub type RequestFilter = Rc<dyn Fn(&str, &str, ResourceKind) -> bool>;

#[derive(Clone, Default)]
pub struct TabHooks {
    pub request_filter: Option<RequestFilter>,
    pub strip_tracking: bool,
    pub login_bridge: bool,
    pub user_scripts: Vec<String>,
    pub page: PageSettings,
    /// Report WebRTC use, for the "who saw your IP" warning.
    pub privacy_probe: bool,
    /// Remembered permission answers; without it WebView2 asks on its own.
    pub permissions: Option<crate::PermissionPolicy>,
}

const PRIVACY_PROBE_JS: &str = include_str!("privacy_probe.js");

/// Settings that can change while a tab is live.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct PageSettings {
    pub block_popups: bool,
    pub tracking: TrackingLevel,
    pub downloads: DownloadTarget,
}

impl Default for PageSettings {
    fn default() -> Self {
        Self {
            block_popups: true,
            tracking: TrackingLevel::Balanced,
            downloads: DownloadTarget::default(),
        }
    }
}

/// One live page: a controller plus its webview. Event handlers hold only
/// the queue and plain data, so dropping this releases everything.
pub struct TabView {
    id: TabId,
    env: ICoreWebView2Environment,
    controller: ICoreWebView2Controller,
    webview: ICoreWebView2,
    queue: EventQueue,
    script_ids: Rc<RefCell<Vec<String>>>,
    nav_target: Rc<RefCell<String>>,
    downloads: downloads::Watch,
    block_popups: Rc<Cell<bool>>,
    document: Rc<RefCell<Option<crate::DocumentResponse>>>,
    permission_requests: crate::permissions::Pending,
}

impl TabView {
    /// Starts creating a controller under `parent`. `on_ready` runs from the
    /// message loop, then `TabEvent::Ready` or `CreateFailed` is queued.
    pub fn create<F>(
        profile: &Profile,
        parent: HWND,
        id: TabId,
        hooks: TabHooks,
        queue: EventQueue,
        on_ready: F,
    ) -> Result<(), EngineError>
    where
        F: FnOnce(Result<TabView, EngineError>) + 'static,
    {
        let env = profile.env.clone();
        let handler = CreateCoreWebView2ControllerCompletedHandler::create(Box::new(
            move |result, controller| {
                let tab = result
                    .map_err(EngineError::from)
                    .and_then(|()| controller.ok_or(EngineError::Missing))
                    .and_then(|c| Self::init(&env, c, id, hooks, queue.clone()));
                let event = match &tab {
                    Ok(_) => TabEvent::Ready,
                    Err(e) => TabEvent::CreateFailed(e.to_string()),
                };
                on_ready(tab);
                queue.push(id, event);
                Ok(())
            },
        ));
        // SAFETY: `parent` is a live window owned by the UI thread.
        unsafe { profile.env.CreateCoreWebView2Controller(parent, &handler)? };
        Ok(())
    }

    fn init(
        env: &ICoreWebView2Environment,
        controller: ICoreWebView2Controller,
        id: TabId,
        hooks: TabHooks,
        queue: EventQueue,
    ) -> Result<Self, EngineError> {
        // SAFETY: the controller was just created on this thread.
        let webview = unsafe { controller.CoreWebView2()? };
        configure(&webview)?;
        let tab = Self {
            id,
            env: env.clone(),
            controller,
            webview,
            queue: queue.clone(),
            script_ids: Rc::default(),
            nav_target: Rc::default(),
            downloads: downloads::Watch {
                tab: id,
                queue: queue.clone(),
                operations: Operations::default(),
                pending: Rc::default(),
                target: Rc::new(RefCell::new(hooks.page.downloads.clone())),
            },
            block_popups: Rc::new(Cell::new(hooks.page.block_popups)),
            document: Rc::default(),
            permission_requests: Rc::default(),
        };
        tab.apply_tracking(hooks.page.tracking);
        tab.watch_navigation(hooks.strip_tracking)?;
        tab.watch_page_state()?;
        tab.watch_popups_and_crashes()?;
        tab.watch_shortcuts()?;
        tab.watch_favicon()?;
        tab.watch_fullscreen()?;
        tab.watch_focus()?;
        crate::find::watch(&tab.webview, id, tab.queue.clone())?;
        // Page info is optional; older runtimes simply have none.
        let _ = crate::inspect::watch_document(
            &tab.webview,
            tab.nav_target.clone(),
            tab.document.clone(),
        );
        downloads::watch(&tab.webview, tab.downloads.clone())?;
        if let Some(filter) = hooks.request_filter {
            tab.install_filter(env.clone(), filter)?;
        }
        if let Some(policy) = hooks.permissions {
            crate::permissions::watch(
                &tab.webview,
                id,
                tab.queue.clone(),
                policy,
                tab.permission_requests.clone(),
            )?;
        }
        if hooks.privacy_probe {
            tab.add_document_script(PRIVACY_PROBE_JS, None)?;
            tab.watch_probe()?;
        }
        if hooks.login_bridge {
            tab.add_document_script(LOGIN_BRIDGE_JS, None)?;
            tab.watch_login()?;
        }
        tab.set_user_scripts(&hooks.user_scripts)?;
        Ok(tab)
    }

    pub fn id(&self) -> TabId {
        self.id
    }

    pub(crate) fn webview(&self) -> &ICoreWebView2 {
        &self.webview
    }

    pub(crate) fn queue_handle(&self) -> EventQueue {
        self.queue.clone()
    }

    pub fn navigate(&self, url: &str) -> Result<(), EngineError> {
        // SAFETY: plain COM calls on the owning thread (applies to every
        // method below).
        unsafe { self.webview.Navigate(&HSTRING::from(url))? };
        Ok(())
    }

    pub fn set_bounds(&self, bounds: PhysicalRect) -> Result<(), EngineError> {
        unsafe { self.controller.SetBounds(bounds.into())? };
        Ok(())
    }

    pub fn set_visible(&self, visible: bool) -> Result<(), EngineError> {
        unsafe { self.controller.SetIsVisible(visible)? };
        Ok(())
    }

    pub fn cancel_download(&self, id: DownloadId) -> Result<(), EngineError> {
        downloads::cancel(&self.downloads.operations, id)
    }

    /// The HTTP response for the page on screen, if it has arrived.
    pub fn document_response(&self) -> Option<crate::DocumentResponse> {
        let url = self.url().ok()?;
        self.document.borrow().clone().filter(|d| d.url == url)
    }

    /// Runs `PAGE_STATS_SCRIPT`; `on_done` gets its JSON, or `None`.
    pub fn page_stats(
        &self,
        on_done: impl FnOnce(Option<String>) + 'static,
    ) -> Result<(), EngineError> {
        self.run_script(crate::PAGE_STATS_SCRIPT, on_done)
    }

    /// Runs `script` in the page's main frame; `on_done` gets the result as
    /// JSON, or `None` if it threw.
    pub fn run_script(
        &self,
        script: &str,
        on_done: impl FnOnce(Option<String>) + 'static,
    ) -> Result<(), EngineError> {
        let script = HSTRING::from(script);
        let handler = ExecuteScriptCompletedHandler::create(Box::new(move |result, json| {
            on_done(result.ok().map(|()| json));
            Ok(())
        }));
        unsafe { self.webview.ExecuteScript(&script, &handler)? };
        Ok(())
    }

    /// Answers `TabEvent::PermissionRequested`. `None` refuses this once.
    pub fn resolve_permission(&self, request: u64, allow: Option<bool>) -> Result<(), EngineError> {
        crate::permissions::resolve(&self.permission_requests, request, allow)
    }

    /// Loads an unpacked extension into this tab's profile (every tab of
    /// the workspace gets it).
    pub fn install_extension(
        &self,
        folder: &std::path::Path,
        on_done: impl FnOnce(Result<crate::ExtensionInfo, String>) + 'static,
    ) -> Result<(), EngineError> {
        crate::extensions::install(&self.webview, folder, on_done)
    }

    pub fn extensions(
        &self,
        on_done: impl FnOnce(Vec<crate::ExtensionInfo>) + 'static,
    ) -> Result<(), EngineError> {
        crate::extensions::list(&self.webview, on_done)
    }

    pub fn set_extension_enabled(&self, id: &str, enabled: bool) -> Result<(), EngineError> {
        crate::extensions::set_enabled(&self.webview, id.to_owned(), enabled)
    }

    pub fn remove_extension(&self, id: &str) -> Result<(), EngineError> {
        crate::extensions::remove(&self.webview, id.to_owned())
    }

    /// Highlights every match of `term` and selects the first.
    pub fn find(&self, term: &str, case_sensitive: bool) -> Result<(), EngineError> {
        crate::find::start(&self.env, &self.webview, term, case_sensitive)
    }

    pub fn find_step(&self, forward: bool) -> Result<(), EngineError> {
        crate::find::step(&self.webview, forward)
    }

    pub fn stop_find(&self) -> Result<(), EngineError> {
        crate::find::stop(&self.webview)
    }

    /// Same id as the frames in `Profile::renderer_memory`.
    pub fn main_frame_id(&self) -> Option<u32> {
        let webview20 = self.webview.cast::<ICoreWebView2_20>().ok()?;
        let mut id = 0;
        unsafe { webview20.FrameId(&mut id) }.ok()?;
        Some(id)
    }

    /// Answers `TabEvent::DownloadSaveAs`: save to `path`, or cancel on `None`.
    pub fn resolve_download(
        &self,
        id: DownloadId,
        path: Option<PathBuf>,
    ) -> Result<(), EngineError> {
        downloads::resolve(&self.downloads, id, path)
    }

    pub fn apply_page_settings(&self, page: &PageSettings) {
        self.block_popups.set(page.block_popups);
        *self.downloads.target.borrow_mut() = page.downloads.clone();
        self.apply_tracking(page.tracking);
    }

    /// Profile-wide, so any one live tab can set it for its workspace.
    /// Older runtimes without ICoreWebView2Profile3 keep their default.
    fn apply_tracking(&self, level: TrackingLevel) {
        let Some(profile) = self.profile() else {
            return;
        };
        let Ok(profile3) = profile.cast::<ICoreWebView2Profile3>() else {
            return;
        };
        let level = match level {
            TrackingLevel::Off => COREWEBVIEW2_TRACKING_PREVENTION_LEVEL_NONE,
            TrackingLevel::Basic => COREWEBVIEW2_TRACKING_PREVENTION_LEVEL_BASIC,
            TrackingLevel::Balanced => COREWEBVIEW2_TRACKING_PREVENTION_LEVEL_BALANCED,
            TrackingLevel::Strict => COREWEBVIEW2_TRACKING_PREVENTION_LEVEL_STRICT,
        };
        let _ = unsafe { profile3.SetPreferredTrackingPreventionLevel(level) };
    }

    /// Clears cookies, storage and cache for this tab's whole profile.
    pub fn clear_site_data(&self, on_done: impl FnOnce(bool) + 'static) -> Result<(), EngineError> {
        let profile2: ICoreWebView2Profile2 = self.profile().ok_or(EngineError::Missing)?.cast()?;
        let kinds =
            COREWEBVIEW2_BROWSING_DATA_KINDS_ALL_SITE | COREWEBVIEW2_BROWSING_DATA_KINDS_DISK_CACHE;
        let handler = ClearBrowsingDataCompletedHandler::create(Box::new(move |result| {
            on_done(result.is_ok());
            Ok(())
        }));
        unsafe { profile2.ClearBrowsingData(kinds, &handler)? };
        Ok(())
    }

    fn profile(&self) -> Option<ICoreWebView2Profile> {
        let webview13 = self.webview.cast::<ICoreWebView2_13>().ok()?;
        unsafe { webview13.Profile() }.ok()
    }

    /// Snapshots the visible page as PNG bytes. Used to keep a dimmed copy
    /// of the page behind chrome that must hide the live view.
    pub fn capture_preview(
        &self,
        on_done: impl FnOnce(Option<Vec<u8>>) + 'static,
    ) -> Result<(), EngineError> {
        // SAFETY: a null HGLOBAL asks COM to allocate and own the buffer.
        let stream = unsafe { CreateStreamOnHGlobal(HGLOBAL::default(), true)? };
        let source = stream.clone();
        let handler = CapturePreviewCompletedHandler::create(Box::new(move |result| {
            on_done(result.ok().and_then(|()| read_stream(&source)));
            Ok(())
        }));
        unsafe {
            self.webview.CapturePreview(
                COREWEBVIEW2_CAPTURE_PREVIEW_IMAGE_FORMAT_PNG,
                &stream,
                &handler,
            )?
        };
        Ok(())
    }

    /// Gives the page keyboard focus.
    pub fn focus(&self) -> Result<(), EngineError> {
        unsafe {
            self.controller
                .MoveFocus(COREWEBVIEW2_MOVE_FOCUS_REASON_PROGRAMMATIC)?
        };
        Ok(())
    }

    /// Call on window move so WebView2 popups (selects, menus) stay attached.
    pub fn notify_parent_moved(&self) -> Result<(), EngineError> {
        unsafe { self.controller.NotifyParentWindowPositionChanged()? };
        Ok(())
    }

    /// `TrySuspend` refuses a visible webview, so hide first. The outcome
    /// arrives as `TabEvent::Suspended`.
    pub fn suspend(&self) -> Result<(), EngineError> {
        self.set_visible(false)?;
        let webview: ICoreWebView2_3 = self.webview.cast()?;
        let (id, queue) = (self.id, self.queue.clone());
        let handler = TrySuspendCompletedHandler::create(Box::new(move |result, ok| {
            queue.push(
                id,
                TabEvent::Suspended {
                    ok: result.is_ok() && ok,
                },
            );
            Ok(())
        }));
        unsafe { webview.TrySuspend(&handler)? };
        Ok(())
    }

    pub fn resume(&self) -> Result<(), EngineError> {
        let webview: ICoreWebView2_3 = self.webview.cast()?;
        unsafe { webview.Resume()? };
        self.set_visible(true)
    }

    pub fn set_muted(&self, muted: bool) -> Result<(), EngineError> {
        let webview: ICoreWebView2_8 = self.webview.cast()?;
        unsafe { webview.SetIsMuted(muted)? };
        Ok(())
    }

    pub fn set_zoom(&self, factor: f64) -> Result<(), EngineError> {
        unsafe { self.controller.SetZoomFactor(factor)? };
        Ok(())
    }

    pub fn reload(&self) -> Result<(), EngineError> {
        unsafe { self.webview.Reload()? };
        Ok(())
    }

    pub fn go_back(&self) -> Result<(), EngineError> {
        unsafe { self.webview.GoBack()? };
        Ok(())
    }

    pub fn go_forward(&self) -> Result<(), EngineError> {
        unsafe { self.webview.GoForward()? };
        Ok(())
    }

    pub fn stop(&self) -> Result<(), EngineError> {
        unsafe { self.webview.Stop()? };
        Ok(())
    }

    pub fn url(&self) -> Result<String, EngineError> {
        Ok(read_pwstr(|p| unsafe { self.webview.Source(p) })?)
    }

    pub fn title(&self) -> Result<String, EngineError> {
        Ok(read_pwstr(|p| unsafe { self.webview.DocumentTitle(p) })?)
    }

    /// Fills the page's login form after the user picked an account. The
    /// script checks the live origin itself before writing anything.
    pub fn fill_login(
        &self,
        origin: &Origin,
        username: &str,
        password: &str,
    ) -> Result<(), EngineError> {
        let script = login::fill_script(origin, username, password);
        self.run_secret_script(&script)
    }

    /// Tells the page's login bridge to show the generated-password offer.
    pub fn offer_password(&self, theme: &login::OfferTheme) -> Result<(), EngineError> {
        let json = HSTRING::from(login::offer_message(theme));
        unsafe { self.webview.PostWebMessageAsJson(&json)? };
        Ok(())
    }

    /// Fills a freshly generated password into the focused new-password
    /// field and its confirm field. The caller drops the password after.
    pub fn fill_generated(&self, origin: &Origin, password: &str) -> Result<(), EngineError> {
        let script = login::generated_fill_script(origin, password);
        self.run_secret_script(&script)
    }

    fn run_secret_script(&self, script: &str) -> Result<(), EngineError> {
        // The UTF-16 copy WebView2 needs cannot be zeroized; it lives only
        // for this call. See decision 0002.
        let wide = HSTRING::from(script);
        let handler = ExecuteScriptCompletedHandler::create(Box::new(|_, _| Ok(())));
        unsafe { self.webview.ExecuteScript(&wide, &handler)? };
        Ok(())
    }

    /// Replaces the userscript bundles. Takes effect from the next navigation.
    pub fn set_user_scripts(&self, bundles: &[String]) -> Result<(), EngineError> {
        for id in self.script_ids.borrow_mut().drain(..) {
            unsafe {
                self.webview
                    .RemoveScriptToExecuteOnDocumentCreated(&HSTRING::from(id))?
            };
        }
        for bundle in bundles {
            self.add_document_script(bundle, Some(self.script_ids.clone()))?;
        }
        Ok(())
    }

    fn add_document_script(
        &self,
        js: &str,
        record: Option<Rc<RefCell<Vec<String>>>>,
    ) -> Result<(), EngineError> {
        let handler = AddScriptToExecuteOnDocumentCreatedCompletedHandler::create(Box::new(
            move |result, script_id| {
                if let (Ok(()), Some(ids)) = (result, record) {
                    ids.borrow_mut().push(script_id);
                }
                Ok(())
            },
        ));
        unsafe {
            self.webview
                .AddScriptToExecuteOnDocumentCreated(&HSTRING::from(js), &handler)?
        };
        Ok(())
    }

    fn watch_navigation(&self, strip: bool) -> Result<(), EngineError> {
        let (id, queue, target) = (self.id, self.queue.clone(), self.nav_target.clone());
        let starting = NavigationStartingEventHandler::create(Box::new(move |sender, args| {
            let (Some(webview), Some(args)) = (sender, args) else {
                return Ok(());
            };
            let url = read_pwstr(|p| unsafe { args.Uri(p) })?;
            if let Some(clean) = strip.then(|| strip_tracking(&url)).flatten() {
                // Cancel and re-navigate; the clean URL has nothing left to strip.
                unsafe { args.SetCancel(true)? };
                unsafe { webview.Navigate(&HSTRING::from(clean))? };
                return Ok(());
            }
            target.replace(url.clone());
            queue.push(id, TabEvent::NavigationStarting { url });
            Ok(())
        }));
        let (id, queue) = (self.id, self.queue.clone());
        let completed = NavigationCompletedEventHandler::create(Box::new(move |_, args| {
            let mut success = BOOL::default();
            if let Some(args) = args {
                unsafe { args.IsSuccess(&mut success)? };
            }
            queue.push(
                id,
                TabEvent::NavigationCompleted {
                    success: success.as_bool(),
                },
            );
            Ok(())
        }));
        let mut token = 0;
        unsafe {
            self.webview.add_NavigationStarting(&starting, &mut token)?;
            self.webview
                .add_NavigationCompleted(&completed, &mut token)?;
        }
        Ok(())
    }

    fn watch_page_state(&self) -> Result<(), EngineError> {
        let (id, queue) = (self.id, self.queue.clone());
        let title = DocumentTitleChangedEventHandler::create(Box::new(move |sender, _| {
            if let Some(webview) = sender {
                let title = read_pwstr(|p| unsafe { webview.DocumentTitle(p) })?;
                queue.push(id, TabEvent::TitleChanged { title });
            }
            Ok(())
        }));
        let (id, queue) = (self.id, self.queue.clone());
        let source = SourceChangedEventHandler::create(Box::new(move |sender, _| {
            if let Some(webview) = sender {
                let url = read_pwstr(|p| unsafe { webview.Source(p) })?;
                queue.push(id, TabEvent::UrlChanged { url });
            }
            Ok(())
        }));
        let (id, queue) = (self.id, self.queue.clone());
        let audio =
            IsDocumentPlayingAudioChangedEventHandler::create(Box::new(move |sender, _| {
                let Some(webview) = sender else {
                    return Ok(());
                };
                let mut playing = BOOL::default();
                unsafe {
                    webview
                        .cast::<ICoreWebView2_8>()?
                        .IsDocumentPlayingAudio(&mut playing)?
                };
                queue.push(
                    id,
                    TabEvent::AudioChanged {
                        playing: playing.as_bool(),
                    },
                );
                Ok(())
            }));
        let mut token = 0;
        unsafe {
            self.webview.add_DocumentTitleChanged(&title, &mut token)?;
            self.webview.add_SourceChanged(&source, &mut token)?;
            let webview8: ICoreWebView2_8 = self.webview.cast()?;
            webview8.add_IsDocumentPlayingAudioChanged(&audio, &mut token)?;
        }
        Ok(())
    }

    fn watch_popups_and_crashes(&self) -> Result<(), EngineError> {
        let (id, queue, block) = (self.id, self.queue.clone(), self.block_popups.clone());
        // Popups become tabs in our strip instead of bare WebView2 windows.
        let popup = NewWindowRequestedEventHandler::create(Box::new(move |_, args| {
            let Some(args) = args else {
                return Ok(());
            };
            let url = read_pwstr(|p| unsafe { args.Uri(p) })?;
            unsafe { args.SetHandled(true)? };
            let mut user = BOOL::default();
            unsafe { args.IsUserInitiated(&mut user)? };
            // Windows opened without a click are almost always ads.
            let event = if block.get() && !user.as_bool() {
                TabEvent::PopupBlocked { url }
            } else {
                TabEvent::NewWindowRequested { url }
            };
            queue.push(id, event);
            Ok(())
        }));
        let (id, queue) = (self.id, self.queue.clone());
        let crashed = ProcessFailedEventHandler::create(Box::new(move |_, _| {
            queue.push(id, TabEvent::ProcessFailed);
            Ok(())
        }));
        let mut token = 0;
        unsafe {
            self.webview.add_NewWindowRequested(&popup, &mut token)?;
            self.webview.add_ProcessFailed(&crashed, &mut token)?;
        }
        Ok(())
    }

    /// Older runtimes without ICoreWebView2_15 simply never report icons.
    fn watch_favicon(&self) -> Result<(), EngineError> {
        let Ok(webview15) = self.webview.cast::<ICoreWebView2_15>() else {
            return Ok(());
        };
        let (id, queue) = (self.id, self.queue.clone());
        let handler = FaviconChangedEventHandler::create(Box::new(move |sender, _| {
            let Some(webview) = sender.and_then(|w| w.cast::<ICoreWebView2_15>().ok()) else {
                return Ok(());
            };
            let queue = queue.clone();
            let done = GetFaviconCompletedHandler::create(Box::new(move |result, stream| {
                let png = match (result, stream) {
                    (Ok(()), Some(stream)) => read_all(&stream),
                    _ => Vec::new(),
                };
                queue.push(id, TabEvent::FaviconChanged { png });
                Ok(())
            }));
            unsafe { webview.GetFavicon(COREWEBVIEW2_FAVICON_IMAGE_FORMAT_PNG, &done)? };
            Ok(())
        }));
        let mut token = 0;
        unsafe { webview15.add_FaviconChanged(&handler, &mut token)? };
        Ok(())
    }

    /// The page lives in another process, so the chrome never sees a blur
    /// when the user clicks into it; this tells it the page took focus.
    fn watch_focus(&self) -> Result<(), EngineError> {
        let (id, queue) = (self.id, self.queue.clone());
        let handler = FocusChangedEventHandler::create(Box::new(move |_, _| {
            queue.push(id, TabEvent::PageFocused);
            Ok(())
        }));
        let mut token = 0;
        unsafe { self.controller.add_GotFocus(&handler, &mut token)? };
        Ok(())
    }

    /// A video's fullscreen button only fills the webview; the chrome hears
    /// about it here and makes the whole window fullscreen.
    fn watch_fullscreen(&self) -> Result<(), EngineError> {
        let (id, queue) = (self.id, self.queue.clone());
        let handler =
            ContainsFullScreenElementChangedEventHandler::create(Box::new(move |sender, _| {
                let Some(webview) = sender else {
                    return Ok(());
                };
                let mut on = BOOL::default();
                unsafe { webview.ContainsFullScreenElement(&mut on)? };
                queue.push(id, TabEvent::FullscreenChanged { on: on.as_bool() });
                Ok(())
            }));
        let mut token = 0;
        unsafe {
            self.webview
                .add_ContainsFullScreenElementChanged(&handler, &mut token)?
        };
        Ok(())
    }

    fn watch_shortcuts(&self) -> Result<(), EngineError> {
        let (id, queue) = (self.id, self.queue.clone());
        let handler = AcceleratorKeyPressedEventHandler::create(Box::new(move |_, args| {
            let Some(args) = args else {
                return Ok(());
            };
            let mut kind = COREWEBVIEW2_KEY_EVENT_KIND::default();
            let mut vk = 0u32;
            unsafe {
                args.KeyEventKind(&mut kind)?;
                args.VirtualKey(&mut vk)?;
            }
            if let Some(shortcut) = shortcut_for_event(kind, vk) {
                if shortcut != shortcuts::Shortcut::CtrlReleased {
                    unsafe { args.SetHandled(true)? };
                }
                queue.push(id, TabEvent::Shortcut(shortcut));
            }
            Ok(())
        }));
        let mut token = 0;
        unsafe {
            self.controller
                .add_AcceleratorKeyPressed(&handler, &mut token)?
        };
        Ok(())
    }

    fn install_filter(
        &self,
        env: ICoreWebView2Environment,
        filter: RequestFilter,
    ) -> Result<(), EngineError> {
        add_request_filter(&self.webview)?;
        let target = self.nav_target.clone();
        let handler = WebResourceRequestedEventHandler::create(Box::new(move |sender, args| {
            let (Some(webview), Some(args)) = (sender, args) else {
                return Ok(());
            };
            let request = unsafe { args.Request()? };
            let url = read_pwstr(|p| unsafe { request.Uri(p) })?;
            let page = read_pwstr(|p| unsafe { webview.Source(p) })?;
            let mut context = COREWEBVIEW2_WEB_RESOURCE_CONTEXT::default();
            unsafe { args.ResourceContext(&mut context)? };
            let kind = resource_kind(context, &url, &target.borrow());
            if filter(&url, &page, kind) {
                let response = unsafe {
                    env.CreateWebResourceResponse(None::<&IStream>, 403, w!("Blocked"), w!(""))?
                };
                unsafe { args.SetResponse(&response)? };
            }
            Ok(())
        }));
        let mut token = 0;
        unsafe {
            self.webview
                .add_WebResourceRequested(&handler, &mut token)?
        };
        Ok(())
    }

    fn watch_probe(&self) -> Result<(), EngineError> {
        let (id, queue) = (self.id, self.queue.clone());
        let handler = WebMessageReceivedEventHandler::create(Box::new(move |_, args| {
            let Some(args) = args else {
                return Ok(());
            };
            let json = read_pwstr(|p| unsafe { args.WebMessageAsJson(p) })?;
            let message: serde_json::Value = serde_json::from_str(&json).unwrap_or_default();
            if message["umbrella"] == "webrtc" {
                queue.push(id, TabEvent::WebRtcUsed);
            }
            Ok(())
        }));
        let mut token = 0;
        unsafe { self.webview.add_WebMessageReceived(&handler, &mut token)? };
        Ok(())
    }

    fn watch_login(&self) -> Result<(), EngineError> {
        let (id, queue) = (self.id, self.queue.clone());
        let handler = WebMessageReceivedEventHandler::create(Box::new(move |_, args| {
            let Some(args) = args else {
                return Ok(());
            };
            let json = Zeroizing::new(read_pwstr(|p| unsafe { args.WebMessageAsJson(p) })?);
            let Some(message) = login::parse_message(&json) else {
                return Ok(());
            };
            // The sender's URL as WebView2 saw it, not anything the page claims.
            let source = read_pwstr(|p| unsafe { args.Source(p) })?;
            let Ok(origin) = Origin::parse(&source) else {
                return Ok(());
            };
            let event = match message {
                LoginMessage::FormSeen => TabEvent::LoginFormSeen { origin },
                LoginMessage::Submitted(credential) => {
                    TabEvent::LoginSubmitted { origin, credential }
                }
                LoginMessage::SuggestWanted => TabEvent::PasswordSuggestWanted { origin },
                LoginMessage::SuggestAccepted => TabEvent::PasswordSuggestAccepted { origin },
            };
            queue.push(id, event);
            Ok(())
        }));
        let mut token = 0;
        unsafe { self.webview.add_WebMessageReceived(&handler, &mut token)? };
        Ok(())
    }
}

impl Drop for TabView {
    fn drop(&mut self) {
        // Close tears down the renderer and releases every handler we added.
        // An error here means it is already gone, which is the goal anyway.
        let _ = unsafe { self.controller.Close() };
    }
}

fn configure(webview: &ICoreWebView2) -> Result<(), EngineError> {
    unsafe {
        let settings = webview.Settings()?;
        // The chrome shows link targets itself.
        settings.SetIsStatusBarEnabled(false)?;
        // The vault owns passwords; WebView2's own save bubble would double-prompt.
        if let Ok(settings4) = settings.cast::<ICoreWebView2Settings4>() {
            settings4.SetIsPasswordAutosaveEnabled(false)?;
            settings4.SetIsGeneralAutofillEnabled(false)?;
        }
    }
    Ok(())
}

/// Newer runtimes can include iframe requests, where many ads live; older
/// ones only report requests from the main document.
fn add_request_filter(webview: &ICoreWebView2) -> WinResult<()> {
    unsafe {
        match webview.cast::<ICoreWebView2_22>() {
            Ok(webview22) => webview22.AddWebResourceRequestedFilterWithRequestSourceKinds(
                w!("*"),
                COREWEBVIEW2_WEB_RESOURCE_CONTEXT_ALL,
                COREWEBVIEW2_WEB_RESOURCE_REQUEST_SOURCE_KINDS_ALL,
            ),
            Err(_) => webview
                .AddWebResourceRequestedFilter(w!("*"), COREWEBVIEW2_WEB_RESOURCE_CONTEXT_ALL),
        }
    }
}

/// WebView2 reports top-level pages and iframes both as DOCUMENT; only the
/// URL of the navigation in flight is the top-level page.
fn resource_kind(
    context: COREWEBVIEW2_WEB_RESOURCE_CONTEXT,
    url: &str,
    nav_target: &str,
) -> ResourceKind {
    match context {
        COREWEBVIEW2_WEB_RESOURCE_CONTEXT_DOCUMENT if url == nav_target => ResourceKind::Document,
        COREWEBVIEW2_WEB_RESOURCE_CONTEXT_DOCUMENT => ResourceKind::Subdocument,
        COREWEBVIEW2_WEB_RESOURCE_CONTEXT_STYLESHEET => ResourceKind::Stylesheet,
        COREWEBVIEW2_WEB_RESOURCE_CONTEXT_SCRIPT => ResourceKind::Script,
        COREWEBVIEW2_WEB_RESOURCE_CONTEXT_IMAGE => ResourceKind::Image,
        COREWEBVIEW2_WEB_RESOURCE_CONTEXT_FONT => ResourceKind::Font,
        COREWEBVIEW2_WEB_RESOURCE_CONTEXT_MEDIA => ResourceKind::Media,
        COREWEBVIEW2_WEB_RESOURCE_CONTEXT_XML_HTTP_REQUEST
        | COREWEBVIEW2_WEB_RESOURCE_CONTEXT_FETCH => ResourceKind::Xhr,
        COREWEBVIEW2_WEB_RESOURCE_CONTEXT_WEBSOCKET => ResourceKind::WebSocket,
        COREWEBVIEW2_WEB_RESOURCE_CONTEXT_PING => ResourceKind::Ping,
        _ => ResourceKind::Other,
    }
}

fn shortcut_for_event(kind: COREWEBVIEW2_KEY_EVENT_KIND, vk: u32) -> Option<shortcuts::Shortcut> {
    let is_up = kind == COREWEBVIEW2_KEY_EVENT_KIND_KEY_UP
        || kind == COREWEBVIEW2_KEY_EVENT_KIND_SYSTEM_KEY_UP;
    if is_up {
        return shortcuts::is_ctrl(vk).then_some(shortcuts::Shortcut::CtrlReleased);
    }
    shortcuts::shortcut_for(vk, held_modifiers())
}

// The event args carry only the key; modifier state comes from the thread's
// keyboard state, which is current while the message is being dispatched.
fn held_modifiers() -> Modifiers {
    let down = |vk: u16| unsafe { GetKeyState(i32::from(vk)) } < 0;
    Modifiers {
        ctrl: down(VK_CONTROL.0),
        shift: down(VK_SHIFT.0),
        alt: down(VK_MENU.0),
    }
}

/// Reads a stream to its end, capped so a hostile page cannot balloon it.
fn read_all(stream: &IStream) -> Vec<u8> {
    const CAP: usize = 512 * 1024;
    let mut out = Vec::new();
    let mut chunk = [0u8; 8192];
    while out.len() < CAP {
        let mut read = 0u32;
        // SAFETY: `chunk` is valid for its length; `read` receives the count.
        let hr = unsafe {
            stream.Read(
                chunk.as_mut_ptr().cast(),
                chunk.len() as u32,
                Some(&mut read),
            )
        };
        if hr.is_err() || read == 0 {
            break;
        }
        out.extend_from_slice(&chunk[..read as usize]);
    }
    out
}

/// Copies the bytes written to an HGLOBAL-backed stream.
fn read_stream(stream: &IStream) -> Option<Vec<u8>> {
    let mut stat = STATSTG::default();
    // SAFETY: Stat fills `stat`; with STATFLAG_NONAME no name is allocated.
    unsafe { stream.Stat(&mut stat, STATFLAG_NONAME) }.ok()?;
    let len = usize::try_from(stat.cbSize).ok()?;
    // SAFETY: the stream owns its HGLOBAL; we lock it only for the copy.
    unsafe {
        let memory = GetHGlobalFromStream(stream).ok()?;
        let ptr = GlobalLock(memory) as *const u8;
        if ptr.is_null() {
            return None;
        }
        let bytes = std::slice::from_raw_parts(ptr, len).to_vec();
        let _ = GlobalUnlock(memory);
        Some(bytes)
    }
}

/// Runs a COM getter that returns a CoTaskMem string and takes ownership.
fn read_pwstr(get: impl FnOnce(*mut PWSTR) -> WinResult<()>) -> WinResult<String> {
    let mut raw = PWSTR::null();
    get(&mut raw)?;
    Ok(take_pwstr(raw))
}
