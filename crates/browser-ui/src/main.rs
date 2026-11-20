//! Umbrella: native GPUI chrome around WebView2.

#![cfg_attr(not(debug_assertions), windows_subsystem = "windows")]

#[cfg(windows)]
mod actions;
#[cfg(windows)]
mod browser;
#[cfg(windows)]
mod chrome;
#[cfg(windows)]
mod downloads_sheet;
#[cfg(windows)]
mod downloads_ui;
#[cfg(windows)]
mod extensions_ui;
#[cfg(windows)]
mod find_bar;
#[cfg(windows)]
mod import_ui;
#[cfg(windows)]
mod instance;
mod motion;
#[cfg(windows)]
mod omnibox;
#[cfg(windows)]
mod page_info;
#[cfg(windows)]
mod paths;
#[cfg(windows)]
mod permissions_ui;
#[cfg(windows)]
mod platform;
#[cfg(windows)]
mod reader;
#[cfg(windows)]
mod settings_ui;
#[cfg(windows)]
mod start_page;
#[cfg(windows)]
mod style;
mod suggest;
#[cfg(windows)]
mod theme_editor;
#[cfg(windows)]
mod vault_ui;

#[cfg(not(windows))]
fn main() {
    eprintln!("Umbrella runs on Windows 10/11 with the WebView2 runtime.");
}

#[cfg(windows)]
fn main() {
    app::run();
}

#[cfg(windows)]
mod app {
    use gpui::{
        App, AppContext, Bounds, QuitMode, TitlebarOptions, WindowBounds, WindowOptions, px, size,
    };
    use gpui_ce_elements::editable_text::actions::{DEFAULT_INPUT_CONTEXT, default_bindings};

    use crate::actions;
    use crate::browser::Browser;
    use crate::instance::{self, Incoming, Launch};
    use crate::paths::Paths;

    pub fn run() {
        let paths = Paths::resolve();
        let args: Vec<String> = std::env::args().skip(1).collect();
        let incoming = match instance::acquire(paths.root(), &args) {
            Ok(Launch::Primary(incoming)) => incoming,
            // Another window owns this profile and now has our arguments.
            Ok(Launch::Forwarded) => return,
            Err(e) => fatal(&format!(
                "Umbrella is already running but did not answer: {e}"
            )),
        };
        gpui_ce_platform::application()
            .with_quit_mode(QuitMode::LastWindowClosed)
            .run(move |cx| open_main_window(cx, incoming));
    }

    fn open_main_window(cx: &mut App, incoming: Incoming) {
        cx.bind_keys(default_bindings().as_keybindings(Some(DEFAULT_INPUT_CONTEXT)));
        actions::bind(cx);
        let bounds = Bounds::centered(None, size(px(1280.0), px(820.0)), cx);
        let options = WindowOptions {
            window_bounds: Some(WindowBounds::Windowed(bounds)),
            titlebar: Some(TitlebarOptions {
                title: Some("Umbrella".into()),
                appears_transparent: true,
                ..Default::default()
            }),
            window_min_size: Some(size(px(480.0), px(320.0))),
            ..Default::default()
        };
        let opened = cx.open_window(options, |window, cx| {
            let browser = cx.new(|cx| match Browser::new(window, cx, incoming) {
                Ok(browser) => browser,
                Err(e) => fatal(&e.to_string()),
            });
            watch_close(&browser, window, cx);
            browser.update(cx, |b, cx| window.focus(&b.focus, cx));
            browser
        });
        if let Err(e) = opened {
            fatal(&format!("cannot open window: {e}"));
        }
        cx.activate(true);
    }

    /// Flush the session before the window goes away.
    fn watch_close(browser: &gpui::Entity<Browser>, window: &mut gpui::Window, cx: &mut App) {
        let browser = browser.downgrade();
        window.on_window_should_close(cx, move |_, cx| {
            let _ = browser.update(cx, |b, _| b.flush());
            true
        });
    }

    fn fatal(message: &str) -> ! {
        eprintln!("Umbrella could not start: {message}");
        std::process::exit(1);
    }
}
