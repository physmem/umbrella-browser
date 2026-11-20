//! Reader mode and the toolbar toast. The reader is drawn inside the page
//! (see js/reader.js); the chrome only toggles it and reports failures.

use std::time::{Duration, Instant};

use gpui::{
    AnyElement, Context, InteractiveElement, IntoElement, ParentElement, SharedString, Styled, div,
    px,
};

use crate::browser::{Browser, NEW_TAB_URL};
use crate::style::{ICON_FONT, icon};

const READER_JS: &str = include_str!("js/reader.js");
const TOAST_SHOWN: Duration = Duration::from_secs(4);
const NO_ARTICLE: &str = "no-article";

impl Browser {
    pub(crate) fn toggle_reader(&mut self, cx: &mut Context<Self>) {
        let Some(view) = self.active_view_ref() else {
            return;
        };
        let t = &self.theme.colors;
        // Theme colours go in as JSON strings, never spliced as raw text.
        let args = serde_json::to_string(&[
            // The window colour can be glass; the reader needs it opaque.
            t.window.with_alpha(1.0).to_hex(),
            t.text.to_hex(),
            t.text_muted.to_hex(),
            t.accent.to_hex(),
        ])
        .unwrap_or_else(|_| "[]".to_owned());
        let script = format!("{READER_JS}(...{args})");
        let (inbox, wake) = (self.script_inbox.clone(), self.wake_handle());
        let started = view.run_script(&script, move |result| {
            if result.as_deref() == Some("null") {
                inbox.borrow_mut().push(NO_ARTICLE.to_owned());
                let _ = wake.unbounded_send(());
            }
        });
        if let Err(e) = started {
            eprintln!("reader unavailable: {e}");
        }
        self.focus_page();
        cx.notify();
    }

    pub(crate) fn apply_script_results(&mut self) {
        let results: Vec<String> = self.script_inbox.borrow_mut().drain(..).collect();
        if results.iter().any(|r| r == NO_ARTICLE) {
            self.show_toast("Reader view needs an article; this page has none.");
        }
    }

    pub(crate) fn show_toast(&mut self, text: &str) {
        self.toast = Some((text.to_owned(), Instant::now()));
    }

    /// True while the toast is up or fading, so the ticker keeps repainting.
    pub(crate) fn toast_visible(&mut self) -> bool {
        let Some((_, since)) = &self.toast else {
            return false;
        };
        if since.elapsed() > TOAST_SHOWN + Duration::from_millis(600) {
            self.toast = None;
        }
        true
    }

    pub(crate) fn toast_pill(&self) -> Option<AnyElement> {
        let live = self
            .toast
            .as_ref()
            .filter(|(_, since)| since.elapsed() < TOAST_SHOWN);
        let shown =
            self.springs
                .borrow_mut()
                .drive("toast", 0.0, f32::from(u8::from(live.is_some())));
        let (text, _) = live?;
        let c = self.colors;
        let text: SharedString = text.clone().into();
        Some(
            div()
                .opacity(shown)
                .ml(px(crate::chrome::RISE * (1.0 - shown)))
                .h(px(24.0))
                .px(px(10.0))
                .flex()
                .items_center()
                .rounded(px(self.theme.shape.radius_sm))
                .bg(c.selection)
                .text_size(px(11.0))
                .child(text)
                .into_any_element(),
        )
    }

    /// The book glyph in the address field.
    pub(crate) fn reader_button(&self, cx: &mut Context<Self>) -> Option<AnyElement> {
        let url = &self.active_tab()?.url;
        let web =
            !self.settings_open && (url.starts_with("http://") || url.starts_with("https://"));
        if !web || url == NEW_TAB_URL {
            return None;
        }
        let c = self.colors;
        Some(
            div()
                .id("reader")
                .size(px(20.0))
                .flex_none()
                .flex()
                .items_center()
                .justify_center()
                .rounded(px(5.0))
                .cursor_pointer()
                .font_family(ICON_FONT)
                .text_size(px(11.0))
                .text_color(c.muted)
                .hover(move |s| s.bg(c.hover))
                .on_mouse_down(
                    gpui::MouseButton::Left,
                    cx.listener(|this, _, _, cx| {
                        cx.stop_propagation();
                        this.toggle_reader(cx);
                    }),
                )
                .child(icon::READER)
                .into_any_element(),
        )
    }
}
