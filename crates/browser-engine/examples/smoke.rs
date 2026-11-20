//! One bare Win32 window with one WebView2 tab, wired through the real
//! engine: adblock (lists in `./filters`), tracking-param stripping,
//! userscripts (`./scripts`) and the login bridge. Events print to stdout.
//!
//! cargo run -p browser-engine --example smoke -- https://example.com

#[cfg(not(windows))]
fn main() {
    eprintln!("the smoke example needs Windows and the WebView2 runtime");
}

#[cfg(windows)]
fn main() -> Result<(), Box<dyn std::error::Error>> {
    app::run()
}

#[cfg(windows)]
mod app {
    use std::cell::RefCell;
    use std::path::Path;
    use std::rc::Rc;

    use browser_core::TabId;
    use browser_engine::{
        EngineEvent, EventQueue, Profile, ProfileOptions, TabEvent, TabHooks, TabView,
    };
    use browser_net::{Blocker, ScriptLibrary};
    use windows::Win32::Foundation::{HWND, LPARAM, LRESULT, RECT, WPARAM};
    use windows::Win32::System::Com::{COINIT_APARTMENTTHREADED, CoInitializeEx};
    use windows::Win32::System::LibraryLoader::GetModuleHandleW;
    use windows::Win32::UI::HiDpi::{
        DPI_AWARENESS_CONTEXT_PER_MONITOR_AWARE_V2, SetProcessDpiAwarenessContext,
    };
    use windows::Win32::UI::WindowsAndMessaging::*;
    use windows::core::{HSTRING, w};

    const TAB: TabId = TabId(1);

    thread_local! {
        static TAB_VIEW: RefCell<Option<TabView>> = const { RefCell::new(None) };
    }

    pub fn run() -> Result<(), Box<dyn std::error::Error>> {
        let url = std::env::args()
            .nth(1)
            .unwrap_or_else(|| "https://example.com/".into());
        // SAFETY: process-wide setup before any window exists.
        unsafe {
            // Per-monitor v2 lets WebView2 follow DPI changes on its own.
            let _ = SetProcessDpiAwarenessContext(DPI_AWARENESS_CONTEXT_PER_MONITOR_AWARE_V2);
            CoInitializeEx(None, COINIT_APARTMENTTHREADED).ok()?;
        }
        let hwnd = create_window()?;
        let hooks = load_hooks()?;
        let queue = EventQueue::new();
        let drain = queue.clone();
        queue.set_waker(move || drain.drain().into_iter().for_each(|e| handle(hwnd, e)));
        start_tab(hwnd, url, hooks, queue)?;
        pump_messages();
        Ok(())
    }

    fn load_hooks() -> Result<TabHooks, Box<dyn std::error::Error>> {
        let blocker = Rc::new(Blocker::load_dir(Path::new("filters"))?);
        let scripts = ScriptLibrary::load_dir(Path::new("scripts"))?;
        println!(
            "adblock rules: {}, userscripts: {}",
            blocker.rule_count(),
            scripts.scripts.len()
        );
        for (path, error) in &scripts.errors {
            println!("skipped {}: {error}", path.display());
        }
        let filter = move |url: &str, page: &str, kind| {
            let block = blocker.should_block(url, page, kind);
            if block {
                println!("blocked {url}");
            }
            block
        };
        Ok(TabHooks {
            request_filter: Some(Rc::new(filter)),
            strip_tracking: true,
            login_bridge: true,
            user_scripts: scripts.bundles(),
            ..TabHooks::default()
        })
    }

    fn start_tab(
        hwnd: HWND,
        url: String,
        hooks: TabHooks,
        queue: EventQueue,
    ) -> Result<(), Box<dyn std::error::Error>> {
        let options = ProfileOptions {
            user_data_dir: std::env::temp_dir().join("umbrella-smoke-profile"),
            language: None,
            extra_args: Vec::new(),
        };
        Profile::create(&options, move |profile| {
            let profile = match profile {
                Ok(p) => p,
                Err(e) => return eprintln!("WebView2 environment failed: {e}"),
            };
            let started =
                TabView::create(&profile, hwnd, TAB, hooks, queue, move |tab| match tab {
                    Ok(tab) => attach(hwnd, tab, &url),
                    Err(e) => eprintln!("tab failed: {e}"),
                });
            if let Err(e) = started {
                eprintln!("tab failed to start: {e}");
            }
        })?;
        Ok(())
    }

    fn attach(hwnd: HWND, tab: TabView, url: &str) {
        if let Err(e) = tab.navigate(url) {
            eprintln!("navigate failed: {e}");
        }
        TAB_VIEW.with_borrow_mut(|slot| *slot = Some(tab));
        fit_to_window(hwnd);
    }

    fn handle(hwnd: HWND, event: EngineEvent) {
        match event.event {
            TabEvent::TitleChanged { title } => {
                println!("title: {title}");
                // SAFETY: `hwnd` is our live top-level window.
                let _ = unsafe {
                    SetWindowTextW(hwnd, &HSTRING::from(format!("{title} - Umbrella smoke")))
                };
            }
            TabEvent::LoginFormSeen { origin } => println!("login form at {origin}"),
            TabEvent::LoginSubmitted { origin, credential } => {
                println!(
                    "login submitted at {origin} for {} (password not shown)",
                    credential.username.as_str()
                );
            }
            other => println!("{:?}: {other:?}", event.tab),
        }
    }

    fn fit_to_window(hwnd: HWND) {
        let mut rect = RECT::default();
        // SAFETY: valid window handle and out pointer.
        if unsafe { GetClientRect(hwnd, &mut rect) }.is_err() {
            return;
        }
        // The client rect is already in device pixels under per-monitor v2.
        let bounds = browser_engine::PhysicalRect {
            left: rect.left,
            top: rect.top,
            right: rect.right,
            bottom: rect.bottom,
        };
        TAB_VIEW.with_borrow(|tab| {
            if let Some(tab) = tab {
                let _ = tab.set_bounds(bounds);
            }
        });
    }

    fn create_window() -> windows::core::Result<HWND> {
        // SAFETY: standard Win32 class registration and window creation.
        unsafe {
            let instance = GetModuleHandleW(None)?;
            let class = WNDCLASSW {
                lpfnWndProc: Some(window_proc),
                hInstance: instance.into(),
                lpszClassName: w!("UmbrellaSmoke"),
                hCursor: LoadCursorW(None, IDC_ARROW)?,
                ..Default::default()
            };
            RegisterClassW(&class);
            let hwnd = CreateWindowExW(
                WINDOW_EX_STYLE::default(),
                w!("UmbrellaSmoke"),
                w!("Umbrella smoke"),
                WS_OVERLAPPEDWINDOW | WS_VISIBLE,
                CW_USEDEFAULT,
                CW_USEDEFAULT,
                1200,
                800,
                None,
                None,
                Some(instance.into()),
                None,
            )?;
            Ok(hwnd)
        }
    }

    extern "system" fn window_proc(
        hwnd: HWND,
        msg: u32,
        wparam: WPARAM,
        lparam: LPARAM,
    ) -> LRESULT {
        match msg {
            WM_SIZE => {
                fit_to_window(hwnd);
                LRESULT(0)
            }
            WM_MOVE => {
                TAB_VIEW.with_borrow(|tab| tab.as_ref().map(|t| t.notify_parent_moved()));
                LRESULT(0)
            }
            WM_DESTROY => {
                // Drop the tab before the window so Close() runs with a live parent.
                TAB_VIEW.with_borrow_mut(|tab| tab.take());
                // SAFETY: ends this thread's message loop.
                unsafe { PostQuitMessage(0) };
                LRESULT(0)
            }
            // SAFETY: default handling for everything else.
            _ => unsafe { DefWindowProcW(hwnd, msg, wparam, lparam) },
        }
    }

    fn pump_messages() {
        let mut msg = MSG::default();
        // SAFETY: classic message loop on the thread that owns the window.
        unsafe {
            while GetMessageW(&mut msg, None, 0, 0).as_bool() {
                let _ = TranslateMessage(&msg);
                DispatchMessageW(&msg);
            }
        }
    }
}
