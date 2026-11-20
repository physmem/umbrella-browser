//! Page info: the main document's HTTP response, a few in-page numbers and
//! a security-header checklist, plus the tab hover card with memory use.

use std::cell::RefCell;
use std::rc::Rc;

use browser_core::TabId;
use browser_engine::DocumentResponse;
use gpui::{
    AnyElement, ClipboardItem, Context, FontWeight, InteractiveElement, IntoElement, ParentElement,
    SharedString, StatefulInteractiveElement, Styled, Window, div, prelude::*, px,
};
use serde::Deserialize;

use crate::browser::{Browser, MemoryReading, NEW_TAB_URL, TabState};
use crate::style::{ICON_FONT, TAB_ROW, icon};

const SHEET_WIDTH: f32 = 640.0;
const CARD_WIDTH: f32 = 240.0;
const MONO_FONT: &str = "Consolas";

/// Headers that tell a browser to protect the page, with what each does.
const SECURITY_HEADERS: &[(&str, &str)] = &[
    (
        "strict-transport-security",
        "Always use HTTPS for this site",
    ),
    (
        "content-security-policy",
        "Limits where scripts and content load from",
    ),
    ("x-frame-options", "Stops other sites framing this page"),
    ("x-content-type-options", "Stops file-type guessing"),
    (
        "referrer-policy",
        "Controls what other sites learn about where you came from",
    ),
    (
        "permissions-policy",
        "Limits camera, microphone and other features",
    ),
];

/// Numbers measured inside the page by `PAGE_STATS_SCRIPT`.
#[derive(Debug, Clone, Default, Deserialize)]
#[serde(default)]
pub(crate) struct PageStats {
    pub protocol: String,
    pub transfer_bytes: u64,
    pub body_bytes: u64,
    pub load_ms: u64,
    pub resources: u32,
    pub script_cookies: u32,
    pub scripts: u32,
    pub iframes: u32,
    pub secure_context: bool,
}

/// What the sheet shows, captured when it opens.
pub(crate) struct PageInfo {
    tab: TabId,
    url: String,
    response: Option<DocumentResponse>,
    stats: Rc<RefCell<Option<PageStats>>>,
    blocked: u32,
}

impl Browser {
    pub(crate) fn open_page_info(&mut self, cx: &mut Context<Self>) {
        if self.page_info_open {
            return self.close_page_info(cx);
        }
        let Some(tab) = self.active_tab().cloned() else {
            return;
        };
        if tab.url == NEW_TAB_URL {
            return;
        }
        let stats: Rc<RefCell<Option<PageStats>>> = Rc::default();
        let response = self.view(tab.id).and_then(|v| v.document_response());
        if let Some(view) = self.view(tab.id) {
            let (slot, wake) = (stats.clone(), self.wake_handle());
            let _ = view.page_stats(move |json| {
                *slot.borrow_mut() = json.and_then(|j| serde_json::from_str(&j).ok());
                let _ = wake.unbounded_send(());
            });
        }
        self.page_info = Some(PageInfo {
            tab: tab.id,
            url: tab.url.clone(),
            response,
            stats,
            blocked: self.blocked_count(),
        });
        self.refresh_tab_memory();
        self.settings_open = false;
        self.downloads_open = false;
        self.omnibox_editing = false;
        self.suggestions.clear();
        self.page_info_open = true;
        self.capture_page_preview();
        self.sync_views();
        self.defer_in_window(cx, |b, window, cx| {
            crate::platform::take_focus(b.hwnd);
            window.focus(&b.focus, cx);
        });
        cx.notify();
    }

    pub(crate) fn close_page_info(&mut self, cx: &mut Context<Self>) {
        if !self.page_info_open {
            return;
        }
        self.page_info_open = false;
        self.sync_views();
        self.focus_page();
        cx.notify();
    }

    /// The lock (or info) glyph in the address field; it opens page info
    /// instead of starting an edit.
    pub(crate) fn site_button(&self, secure: bool, cx: &mut Context<Self>) -> Option<AnyElement> {
        let url = &self.active_tab()?.url;
        if self.settings_open || url == NEW_TAB_URL || url.is_empty() {
            return None;
        }
        let c = self.colors;
        let glyph = if secure { icon::LOCK } else { icon::INFO };
        Some(
            div()
                .id("site-info")
                .size(px(20.0))
                .flex_none()
                .flex()
                .items_center()
                .justify_center()
                .rounded(px(5.0))
                .cursor_pointer()
                .font_family(ICON_FONT)
                .text_size(px(10.0))
                .text_color(if self.page_info_open {
                    c.accent
                } else {
                    c.muted
                })
                .hover(move |s| s.bg(c.hover))
                .on_mouse_down(
                    gpui::MouseButton::Left,
                    cx.listener(|this, _, _, cx| {
                        cx.stop_propagation();
                        this.open_page_info(cx);
                    }),
                )
                .child(glyph)
                .into_any_element(),
        )
    }

    pub(crate) fn page_info_sheet(
        &self,
        window: &Window,
        cx: &mut Context<Self>,
    ) -> Option<AnyElement> {
        let reveal = self.springs.borrow_mut().drive(
            "page-info",
            0.0,
            f32::from(u8::from(self.page_info_open)),
        );
        if !self.page_info_open {
            return None;
        }
        let info = self.page_info.as_ref()?;
        let c = self.colors;
        let height =
            (f32::from(window.viewport_size().height) - crate::style::CHROME_HEIGHT - 32.0)
                .max(120.0);
        let stats = info.stats.borrow().clone();
        let card = div()
            .id("page-info")
            .mt(px(crate::chrome::RISE * 2.0 * (1.0 - reveal)))
            .opacity(reveal)
            .w(px(SHEET_WIDTH))
            .max_h(px(height))
            .overflow_y_scroll()
            .p(px(20.0))
            .flex()
            .flex_col()
            .gap(px(20.0))
            .rounded(px(self.theme.shape.radius_lg))
            .bg(c.capsule)
            .shadow_lg()
            .text_size(px(13.0))
            .child(self.page_info_header(info, cx))
            .child(self.overview(info, stats.as_ref()))
            .child(self.ip_section(info))
            .children(info.response.as_ref().map(|r| self.security_checklist(r)))
            .child(self.header_list(info));
        Some(self.sheet_backdrop(card.into_any_element()))
    }

    fn page_info_header(&self, info: &PageInfo, cx: &mut Context<Self>) -> impl IntoElement {
        let c = self.colors;
        let https = info.url.starts_with("https://");
        let host: SharedString = crate::browser::host_of(&info.url)
            .unwrap_or_else(|| info.url.clone())
            .into();
        let (badge, badge_color) = if https {
            ("Secure connection", c.accent)
        } else {
            (
                "Not secure: anything you send here can be read on the way",
                c.muted,
            )
        };
        div()
            .flex()
            .items_start()
            .gap(px(8.0))
            .child(
                div()
                    .flex_1()
                    .min_w_0()
                    .flex()
                    .flex_col()
                    .gap(px(4.0))
                    .child(
                        div()
                            .text_size(px(16.0))
                            .font_weight(FontWeight::SEMIBOLD)
                            .truncate()
                            .child(host),
                    )
                    .child(
                        div()
                            .flex()
                            .items_center()
                            .gap(px(6.0))
                            .text_size(px(12.0))
                            .text_color(badge_color)
                            .child(
                                div()
                                    .font_family(ICON_FONT)
                                    .text_size(px(10.0))
                                    .child(if https { icon::LOCK } else { icon::INFO }),
                            )
                            .child(badge),
                    ),
            )
            .child(
                action("view-source", "View source", c).on_click(cx.listener(|this, _, _, cx| {
                    let Some(info) = this.page_info.as_ref() else {
                        return;
                    };
                    let (tab, url) = (info.tab, format!("view-source:{}", info.url));
                    this.close_page_info(cx);
                    this.open_tab(&url, Some(tab), true);
                    cx.notify();
                })),
            )
            .child(
                action("copy-headers", "Copy headers", c).on_click(cx.listener(
                    |this, _, _, cx| {
                        if let Some(text) = this.page_info.as_ref().map(PageInfo::headers_text) {
                            cx.write_to_clipboard(ClipboardItem::new_string(text));
                        }
                    },
                )),
            )
            .child(
                div()
                    .id("page-info-close")
                    .size(px(28.0))
                    .flex()
                    .items_center()
                    .justify_center()
                    .rounded(px(6.0))
                    .font_family(ICON_FONT)
                    .text_size(px(10.0))
                    .hover(move |s| s.bg(c.hover))
                    .on_click(cx.listener(|this, _, _, cx| this.close_page_info(cx)))
                    .child(icon::CLOSE),
            )
    }

    fn overview(&self, info: &PageInfo, stats: Option<&PageStats>) -> impl IntoElement {
        let c = self.colors;
        let response = info.response.as_ref();
        let status = response.map_or(
            "Not captured (page loaded before Umbrella was watching)".to_owned(),
            |r| format!("{} {}", r.status, r.reason).trim().to_owned(),
        );
        let mut rows: Vec<(&'static str, String)> = vec![("Status", status)];
        if let Some(s) = stats.filter(|s| !s.protocol.is_empty()) {
            rows.push(("Protocol", protocol_name(&s.protocol)));
        }
        if let Some(v) = response.and_then(|r| r.header("content-type")) {
            rows.push(("Content type", v.to_owned()));
        }
        if let Some(v) = response.and_then(|r| r.header("server")) {
            rows.push(("Server", v.to_owned()));
        }
        if let Some(s) = stats {
            if s.transfer_bytes > 0 || s.body_bytes > 0 {
                rows.push((
                    "Page size",
                    format!(
                        "{} sent, {} unpacked",
                        bytes(s.transfer_bytes),
                        bytes(s.body_bytes)
                    ),
                ));
            }
            if s.load_ms > 0 {
                rows.push(("Load time", format!("{:.2} s", s.load_ms as f64 / 1000.0)));
            }
            rows.push((
                "Content",
                format!(
                    "{} requests, {} scripts, {} frames",
                    s.resources + 1,
                    s.scripts,
                    s.iframes
                ),
            ));
            rows.push(("Cookies scripts can read", s.script_cookies.to_string()));
        }
        rows.push(("Ads and trackers blocked", info.blocked.to_string()));
        if let Some(m) = self.tab_memory.borrow().get(&info.tab) {
            rows.push(("Memory", memory_label(m)));
        }
        section("Overview", c).child(div().flex().flex_col().gap(px(6.0)).children(
            rows.into_iter().map(|(k, v)| {
                let v: SharedString = v.into();
                div()
                    .flex()
                    .gap(px(16.0))
                    .child(div().w(px(180.0)).flex_none().text_color(c.muted).child(k))
                    .child(div().flex_1().min_w_0().child(v))
            }),
        ))
    }

    /// Every server contacted sees the IP; this lists the ones that are not
    /// the site itself, with IP-lookup services called out.
    fn ip_section(&self, info: &PageInfo) -> impl IntoElement {
        let c = self.colors;
        let exposure = self
            .exposure
            .borrow()
            .get(&info.tab)
            .cloned()
            .unwrap_or_default();
        let mut notes: Vec<(bool, String)> = Vec::new();
        for host in &exposure.ip_lookups {
            notes.push((
                true,
                format!(
                    "Looked up your IP with {host}, a service that maps addresses to locations"
                ),
            ));
        }
        if exposure.webrtc {
            notes.push((
                true,
                "Opened a WebRTC connection, which can reveal your real IP even behind a VPN"
                    .to_owned(),
            ));
        }
        let others = exposure.third_parties.len();
        let summary: SharedString = match others {
            0 => "Only this site saw your IP address while the page loaded.".into(),
            1 => "1 other site also saw your IP address while the page loaded.".into(),
            n => format!("{n} other sites also saw your IP address while the page loaded.").into(),
        };
        let blocked: SharedString = format!(
            "{} requests were blocked before they left your PC.",
            exposure.blocked
        )
        .into();
        section("Who saw your IP address", c)
            .child(div().child(summary))
            .children(notes.into_iter().map(|(warn, text)| {
                let text: SharedString = text.into();
                div()
                    .flex()
                    .items_center()
                    .gap(px(8.0))
                    .child(
                        div()
                            .font_family(ICON_FONT)
                            .text_size(px(10.0))
                            .text_color(if warn { c.accent } else { c.muted })
                            .child(icon::INFO),
                    )
                    .child(div().flex_1().min_w_0().child(text))
            }))
            .when(others > 0, |d| {
                d.child(
                    div()
                        .flex()
                        .flex_wrap()
                        .gap(px(6.0))
                        .children(exposure.third_parties.iter().map(|site| {
                            let flagged = exposure.ip_lookups.iter().any(|h| crate::page_info::same_site(h, site));
                            let label: SharedString = site.clone().into();
                            div()
                                .px(px(8.0))
                                .h(px(22.0))
                                .flex()
                                .items_center()
                                .rounded_full()
                                .bg(c.field)
                                .text_size(px(12.0))
                                .when(flagged, |d| d.text_color(c.accent).font_weight(FontWeight::MEDIUM))
                                .child(label)
                        })),
                )
            })
            .child(div().text_size(px(12.0)).text_color(c.muted).child(blocked))
            .child(
                div()
                    .text_size(px(12.0))
                    .text_color(c.muted)
                    .child("No browser can see what a server does with your IP; a VPN hides it from all of them."),
            )
    }

    /// Corner notice in the toolbar when the page looked up the IP address.
    pub(crate) fn ip_pill(&self, cx: &mut Context<Self>) -> Option<AnyElement> {
        let alert = self.ip_alert.filter(|(tab, since)| {
            Some(*tab) == self.active_id() && since.elapsed() < crate::browser::IP_ALERT_SHOWN
        });
        let shown =
            self.springs
                .borrow_mut()
                .drive("ip-pill", 0.0, f32::from(u8::from(alert.is_some())));
        let (tab, _) = alert?;
        let c = self.colors;
        let exposure = self
            .exposure
            .borrow()
            .get(&tab)
            .cloned()
            .unwrap_or_default();
        let label = if exposure.ip_lookups.is_empty() {
            "This page used WebRTC, which can reveal your IP"
        } else {
            "This page looked up your IP"
        };
        Some(
            div()
                .id("ip-alert")
                .opacity(shown)
                .ml(px(crate::chrome::RISE * (1.0 - shown)))
                .h(px(24.0))
                .px(px(8.0))
                .flex()
                .items_center()
                .gap(px(6.0))
                .rounded(px(self.theme.shape.radius_sm))
                .bg(c.selection)
                .text_size(px(11.0))
                .hover(move |s| s.bg(c.hover))
                .active(|s| s.opacity(0.7))
                .on_click(cx.listener(|this, _, _, cx| {
                    this.ip_alert = None;
                    this.open_page_info(cx);
                }))
                .child(
                    div()
                        .font_family(ICON_FONT)
                        .text_size(px(10.0))
                        .text_color(c.accent)
                        .child(icon::INFO),
                )
                .child(label)
                .child(div().text_color(c.accent).child("Details"))
                .into_any_element(),
        )
    }

    fn security_checklist(&self, response: &DocumentResponse) -> impl IntoElement {
        let c = self.colors;
        section("Security headers", c).child(div().flex().flex_col().gap(px(6.0)).children(
            SECURITY_HEADERS.iter().map(|(name, meaning)| {
                let present = response.header(name).is_some();
                div()
                    .flex()
                    .items_center()
                    .gap(px(10.0))
                    .child(
                        div()
                            .w(px(14.0))
                            .flex_none()
                            .font_family(ICON_FONT)
                            .text_size(px(10.0))
                            .text_color(if present { c.accent } else { c.muted })
                            .child(if present { icon::CHECK } else { icon::CLOSE }),
                    )
                    .child(
                        div()
                            .w(px(230.0))
                            .flex_none()
                            .font_family(MONO_FONT)
                            .text_size(px(12.0))
                            .when(!present, |d| d.text_color(c.muted))
                            .child(*name),
                    )
                    .child(
                        div()
                            .flex_1()
                            .min_w_0()
                            .text_size(px(12.0))
                            .text_color(c.muted)
                            .child(*meaning),
                    )
            }),
        ))
    }

    fn header_list(&self, info: &PageInfo) -> impl IntoElement {
        let c = self.colors;
        let Some(response) = info.response.as_ref() else {
            return section("Response headers", c).child(
                div()
                    .text_color(c.muted)
                    .child("Reload the page to capture its response headers."),
            );
        };
        section("Response headers", c).child(
            div()
                .p(px(10.0))
                .rounded(px(self.theme.shape.radius_md))
                .bg(c.field)
                .flex()
                .flex_col()
                .gap(px(3.0))
                .font_family(MONO_FONT)
                .text_size(px(12.0))
                .children(response.headers.iter().map(|(name, value)| {
                    let (name, value): (SharedString, SharedString) =
                        (name.clone().into(), value.clone().into());
                    div()
                        .flex()
                        .gap(px(10.0))
                        .child(
                            div()
                                .w(px(200.0))
                                .flex_none()
                                .text_color(c.accent)
                                .child(name),
                        )
                        .child(div().flex_1().min_w_0().child(value))
                })),
        )
    }

    // ---- tab hover card -----------------------------------------------

    pub(crate) fn on_tab_hover(&mut self, id: TabId, over: bool) {
        if over {
            if self.hovered_tab.is_none_or(|(t, _)| t != id) {
                // Moving between tabs keeps the card up without a new delay.
                let since = match self.hovered_tab {
                    Some((_, since)) => since,
                    None => std::time::Instant::now(),
                };
                self.hovered_tab = Some((id, since));
            }
            self.refresh_tab_memory();
        } else if self.hovered_tab.is_some_and(|(t, _)| t == id) {
            self.hovered_tab = None;
        }
    }

    /// A small card under the hovered tab: its site and memory. It sits over
    /// the toolbar because nothing can be drawn over the page.
    pub(crate) fn tab_hover_card(&self) -> Option<AnyElement> {
        let due = self
            .hovered_tab
            .is_some_and(|(_, since)| since.elapsed() >= std::time::Duration::from_millis(450));
        let dragging = self.tab_drag.is_some_and(|d| d.moved);
        let shown =
            self.springs
                .borrow_mut()
                .drive("tab-card", 0.0, f32::from(u8::from(due && !dragging)));
        let (id, _) = self.hovered_tab?;
        if shown <= 0.01 {
            return None;
        }
        let tab = self.workspace()?.tabs.iter().find(|t| t.id == id)?.clone();
        let slot = self.motion.slot(id)?;
        let c = self.colors;
        let host: SharedString = if tab.url == NEW_TAB_URL {
            "Start page".into()
        } else {
            crate::browser::host_of(&tab.url)
                .unwrap_or_else(|| tab.url.clone())
                .into()
        };
        let memory: SharedString = match self.tab_state(id) {
            _ if tab.url == NEW_TAB_URL => "No page, no memory".into(),
            TabState::NotLoaded => "Not loaded yet".into(),
            TabState::Asleep => "Asleep, memory handed back".into(),
            TabState::Live => self
                .tab_memory
                .borrow()
                .get(&id)
                .map_or("Measuring memory…".to_owned(), memory_label)
                .into(),
        };
        Some(
            div()
                .absolute()
                .left(px(slot.x))
                .top(px(TAB_ROW + 2.0 - crate::chrome::RISE * 0.5 * (1.0 - shown)))
                .w(px(CARD_WIDTH.max(slot.width)))
                .h(px(34.0))
                .px(px(10.0))
                .flex()
                .items_center()
                .gap(px(8.0))
                .opacity(shown)
                .rounded(px(self.theme.shape.radius_md))
                .bg(c.capsule)
                .border_1()
                .border_color(c.border)
                .shadow_md()
                .text_size(px(12.0))
                .child(
                    div()
                        .flex_1()
                        .min_w_0()
                        .truncate()
                        .text_color(c.muted)
                        .child(host),
                )
                .child(
                    div()
                        .flex_none()
                        .font_weight(FontWeight::MEDIUM)
                        .child(memory),
                )
                .into_any_element(),
        )
    }
}

impl PageInfo {
    fn headers_text(&self) -> String {
        let Some(r) = &self.response else {
            return self.url.clone();
        };
        let mut text = format!("{}\n{} {}\n", r.url, r.status, r.reason);
        for (name, value) in &r.headers {
            text.push_str(&format!("{name}: {value}\n"));
        }
        text
    }
}

/// True when `host` belongs to `site` (`api.ipify.org` and `ipify.org`).
pub(crate) fn same_site(host: &str, site: &str) -> bool {
    host == site || host.ends_with(&format!(".{site}"))
}

fn memory_label(m: &MemoryReading) -> String {
    let size = bytes(m.bytes);
    if m.shared > 1 {
        format!("{size} memory, shared by {} tabs", m.shared)
    } else {
        format!("{size} memory")
    }
}

fn protocol_name(protocol: &str) -> String {
    match protocol {
        "h3" => "HTTP/3".to_owned(),
        "h2" => "HTTP/2".to_owned(),
        "http/1.1" => "HTTP/1.1".to_owned(),
        other => other.to_owned(),
    }
}

/// 1536 -> "1.5 KB", 157286400 -> "150 MB".
fn bytes(n: u64) -> String {
    const KB: f64 = 1024.0;
    let n = n as f64;
    if n >= KB * KB * KB {
        format!("{:.1} GB", n / (KB * KB * KB))
    } else if n >= KB * KB {
        format!("{:.0} MB", n / (KB * KB))
    } else if n >= KB {
        format!("{:.1} KB", n / KB)
    } else {
        format!("{n:.0} B")
    }
}

fn section(title: &'static str, c: crate::style::ChromeColors) -> gpui::Div {
    div().flex().flex_col().gap(px(8.0)).child(
        div()
            .text_size(px(12.0))
            .font_weight(FontWeight::SEMIBOLD)
            .text_color(c.muted)
            .child(title),
    )
}

fn action(
    id: &'static str,
    label: &'static str,
    c: crate::style::ChromeColors,
) -> gpui::Stateful<gpui::Div> {
    div()
        .id(id)
        .flex_none()
        .px(px(10.0))
        .h(px(26.0))
        .flex()
        .items_center()
        .rounded(px(6.0))
        .border_1()
        .border_color(c.border)
        .text_size(px(12.0))
        .font_weight(FontWeight::MEDIUM)
        .hover(move |s| s.bg(c.hover))
        .active(|s| s.opacity(0.7))
        .child(label)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn sizes_read_naturally() {
        assert_eq!(bytes(512), "512 B");
        assert_eq!(bytes(1536), "1.5 KB");
        assert_eq!(bytes(150 * 1024 * 1024), "150 MB");
    }

    #[test]
    fn stats_parse_with_missing_fields() {
        let s: PageStats =
            serde_json::from_str(r#"{"protocol":"h2","load_ms":812}"#).unwrap_or_default();
        assert_eq!(protocol_name(&s.protocol), "HTTP/2");
        assert_eq!(s.load_ms, 812);
        assert_eq!(s.scripts, 0);
    }
}
