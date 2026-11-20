//! The permission prompt in the toolbar and the remembered-decisions list in
//! settings. The prompt sits in the toolbar because nothing can be drawn
//! over the page.

use gpui::{
    AnyElement, Context, FontWeight, InteractiveElement, IntoElement, ParentElement, SharedString,
    StatefulInteractiveElement, Styled, div, prelude::*, px,
};

use crate::browser::Browser;
use crate::style::{ICON_FONT, icon};

impl Browser {
    pub(crate) fn permission_pill(&self, cx: &mut Context<Self>) -> Option<AnyElement> {
        let prompt = self.active_permission_prompt();
        let shown = self.springs.borrow_mut().drive(
            "permission-pill",
            0.0,
            f32::from(u8::from(prompt.is_some())),
        );
        let prompt = prompt?;
        let c = self.colors;
        let host = crate::browser::host_of(&prompt.origin).unwrap_or_else(|| prompt.origin.clone());
        let text: SharedString = format!("{host} wants to {}", prompt.kind.request_text()).into();
        let request = prompt.request;
        Some(
            div()
                .id("permission-pill")
                .opacity(shown)
                .ml(px(crate::chrome::RISE * (1.0 - shown)))
                .h(px(28.0))
                .max_w(px(360.0))
                .pl(px(10.0))
                .pr(px(4.0))
                .flex()
                .items_center()
                .gap(px(6.0))
                .rounded(px(self.theme.shape.radius_lg))
                .bg(c.capsule)
                .border_1()
                .border_color(c.border)
                .text_size(px(12.0))
                .child(div().flex_1().min_w_0().truncate().child(text))
                .child(
                    pill_button("permission-block", "Block", false, c).on_click(cx.listener(
                        move |this, _, _, cx| {
                            this.answer_permission(request, Some(false));
                            cx.notify();
                        },
                    )),
                )
                .child(
                    pill_button("permission-allow", "Allow", true, c).on_click(cx.listener(
                        move |this, _, _, cx| {
                            this.answer_permission(request, Some(true));
                            cx.notify();
                        },
                    )),
                )
                .child(
                    div()
                        .id("permission-dismiss")
                        .size(px(20.0))
                        .flex()
                        .items_center()
                        .justify_center()
                        .rounded(px(5.0))
                        .font_family(ICON_FONT)
                        .text_size(px(8.0))
                        .hover(move |s| s.bg(c.hover))
                        .on_click(cx.listener(move |this, _, _, cx| {
                            this.answer_permission(request, None);
                            cx.notify();
                        }))
                        .child(icon::CLOSE),
                )
                .into_any_element(),
        )
    }

    /// Remembered answers with a way to forget each one.
    pub(crate) fn permission_list(&self, cx: &mut Context<Self>) -> AnyElement {
        let c = self.colors;
        let decisions = self.permissions.all();
        if decisions.is_empty() {
            return div()
                .text_size(px(12.0))
                .text_color(c.muted)
                .child("Sites you allow or block show up here.")
                .into_any_element();
        }
        div()
            .flex()
            .flex_col()
            .gap(px(4.0))
            .children(decisions.into_iter().enumerate().map(|(i, d)| {
                let host: SharedString = crate::browser::host_of(&d.origin)
                    .unwrap_or_else(|| d.origin.clone())
                    .into();
                let (origin, kind) = (d.origin.clone(), d.kind);
                div()
                    .flex()
                    .items_center()
                    .gap(px(12.0))
                    .child(div().flex_1().min_w_0().truncate().child(host))
                    .child(div().w(px(140.0)).text_color(c.muted).child(d.kind.label()))
                    .child(
                        div()
                            .w(px(56.0))
                            .text_color(if d.allow { c.accent } else { c.muted })
                            .font_weight(FontWeight::MEDIUM)
                            .child(if d.allow { "Allowed" } else { "Blocked" }),
                    )
                    .child(
                        self.action_button(("forget-permission", i), "Forget", false, cx)
                            .on_click(cx.listener(move |this, _, _, cx| {
                                let _ = this.permissions.forget(&origin, kind);
                                cx.notify();
                            })),
                    )
            }))
            .into_any_element()
    }
}

fn pill_button(
    id: impl Into<gpui::ElementId>,
    label: &'static str,
    primary: bool,
    c: crate::style::ChromeColors,
) -> gpui::Stateful<gpui::Div> {
    div()
        .id(id)
        .flex_none()
        .px(px(10.0))
        .h(px(22.0))
        .flex()
        .items_center()
        .rounded(px(6.0))
        .text_size(px(12.0))
        .font_weight(FontWeight::MEDIUM)
        .when(primary, |d| d.bg(c.accent).text_color(c.on_accent))
        .when(!primary, |d| d.border_1().border_color(c.border))
        .hover(|s| s.opacity(0.85))
        .active(|s| s.opacity(0.7))
        .child(label)
}
