//! Find in page. The bar is a row under the toolbar, Safari-style; the page
//! moves down to make room since nothing can be drawn over it.

use gpui::{
    AnyElement, Context, Focusable, FontWeight, InteractiveElement, IntoElement, ParentElement,
    SharedString, StatefulInteractiveElement, Styled, Window, div, prelude::*, px,
};
use gpui_ce_elements::editable_text::actions::{Enter, Escape};
use gpui_ce_elements::editable_text::text_input;

use crate::browser::{Browser, NEW_TAB_URL};
use crate::style::{ICON_FONT, icon};

pub(crate) const FIND_ROW: f32 = 36.0;

impl Browser {
    pub(crate) fn open_find(&mut self, window: &mut Window, cx: &mut Context<Self>) {
        let Some(tab) = self.active_tab() else {
            return;
        };
        if tab.url == NEW_TAB_URL || self.settings_open {
            return;
        }
        self.end_omnibox_editing(cx);
        self.find_open = true;
        crate::platform::take_focus(self.hwnd);
        let handle = self.find_field.read(cx).focus_handle(cx);
        window.focus(&handle, cx);
        self.find_field
            .update(cx, |field, cx| field.select_document(cx));
        // Reopening keeps the old term; show its matches again.
        self.run_find(cx);
        cx.notify();
    }

    pub(crate) fn close_find(&mut self, cx: &mut Context<Self>) {
        if !self.find_open {
            return;
        }
        self.find_open = false;
        self.find_result = (0, 0);
        self.with_active(|v| {
            let _ = v.stop_find();
        });
        self.focus_page();
        cx.notify();
    }

    /// Searches the active page for the field's text.
    pub(crate) fn run_find(&mut self, cx: &mut Context<Self>) {
        if !self.find_open {
            return;
        }
        let term = self.find_field.read(cx).as_str().to_owned();
        let case = self.find_case;
        if term.is_empty() {
            self.find_result = (0, 0);
            self.with_active(|v| {
                let _ = v.stop_find();
            });
        } else {
            self.with_active(|v| {
                if let Err(e) = v.find(&term, case) {
                    eprintln!("find unavailable: {e}");
                }
            });
        }
        cx.notify();
    }

    fn find_step(&mut self, forward: bool) {
        self.with_active(|v| {
            let _ = v.find_step(forward);
        });
    }

    pub(crate) fn find_bar(&self, cx: &mut Context<Self>) -> Option<AnyElement> {
        if !self.find_open {
            return None;
        }
        let c = self.colors;
        let term_empty = self.find_field.read(cx).as_str().is_empty();
        let (active, count) = self.find_result;
        let status: SharedString = match (term_empty, count) {
            (true, _) => "".into(),
            (false, 0) => "No matches".into(),
            (false, n) => format!("{} of {n}", active.max(1)).into(),
        };
        let field = div()
            .w(px(300.0))
            .h(px(26.0))
            .px(px(8.0))
            .flex()
            .items_center()
            .gap(px(6.0))
            .rounded(px(self.theme.shape.radius_md))
            .bg(c.field)
            .border_1()
            .border_color(c.focus)
            .child(
                div()
                    .font_family(ICON_FONT)
                    .text_size(px(10.0))
                    .text_color(c.muted)
                    .child(icon::SEARCH),
            )
            .child(
                text_input("find-input")
                    .state(self.find_field.downgrade())
                    .placeholder("Find on page")
                    .placeholder_color(c.muted)
                    .caret_color(c.accent)
                    .caret_blink_interval_500ms()
                    .flex_1()
                    .whitespace_nowrap(),
            )
            .child(
                div()
                    .flex_none()
                    .text_size(px(11.0))
                    .text_color(if count == 0 && !term_empty {
                        c.accent
                    } else {
                        c.muted
                    })
                    .child(status),
            );
        Some(
            div()
                .id("find-bar")
                .h(px(FIND_ROW))
                .w_full()
                .px(px(12.0))
                .flex()
                .items_center()
                .gap(px(6.0))
                .border_t_1()
                .border_color(c.border)
                .text_size(px(12.0))
                // Enter steps through matches here instead of submitting the
                // omnibox; Esc closes the bar instead of the panels.
                .on_action(cx.listener(|this, _: &Enter, window, cx| {
                    cx.stop_propagation();
                    this.find_step(!window.modifiers().shift);
                }))
                .on_action(cx.listener(|this, _: &Escape, _, cx| {
                    cx.stop_propagation();
                    this.close_find(cx);
                }))
                .child(field)
                .child(self.find_button("find-prev", icon::UP, cx, |this, _| this.find_step(false)))
                .child(
                    self.find_button("find-next", icon::DOWN, cx, |this, _| this.find_step(true)),
                )
                .child(
                    div()
                        .id("find-case")
                        .h(px(26.0))
                        .px(px(8.0))
                        .flex()
                        .items_center()
                        .rounded(px(6.0))
                        .font_weight(FontWeight::MEDIUM)
                        .when(self.find_case, |d| d.bg(c.selection).text_color(c.accent))
                        .when(!self.find_case, |d| d.text_color(c.muted))
                        .hover(move |s| s.bg(c.hover))
                        .on_click(cx.listener(|this, _, _, cx| {
                            this.find_case = !this.find_case;
                            this.run_find(cx);
                        }))
                        .child("Match case"),
                )
                .child(div().flex_1())
                .child(self.find_button("find-close", icon::CLOSE, cx, |this, cx| {
                    this.close_find(cx)
                }))
                .into_any_element(),
        )
    }

    fn find_button(
        &self,
        id: &'static str,
        glyph: &'static str,
        cx: &mut Context<Self>,
        act: impl Fn(&mut Browser, &mut Context<Browser>) + 'static,
    ) -> impl IntoElement {
        let c = self.colors;
        div()
            .id(id)
            .size(px(26.0))
            .flex()
            .items_center()
            .justify_center()
            .rounded(px(6.0))
            .font_family(ICON_FONT)
            .text_size(px(10.0))
            .hover(move |s| s.bg(c.hover))
            .active(|s| s.opacity(0.6))
            .on_click(cx.listener(move |this, _, _, cx| {
                act(this, cx);
                cx.notify();
            }))
            .child(glyph)
    }
}
