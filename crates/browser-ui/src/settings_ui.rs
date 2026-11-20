//! Settings, opened as a tab of its own. The page area holds a sidebar of
//! panes and a single scrolling column of grouped rows, Windows 11 style.
//! Nothing here is drawn over a live page: the page view is hidden while
//! Settings is the current tab.

use browser_core::ThemeId;
use browser_core::settings::{Startup, TrackingLevel};
use browser_core::vault::generator::{self, GeneratorOptions};
use gpui::{
    AnyElement, ColorExt, Context, FontWeight, InteractiveElement, IntoElement, ParentElement,
    SharedString, StatefulInteractiveElement, Styled, Window, div, img, prelude::*, px,
};
use gpui::{MouseButton, canvas};

use crate::browser::Browser;
use crate::style::{ICON_FONT, icon, mix};

const SIDEBAR_WIDTH: f32 = 232.0;
const NAV_ROW: f32 = 40.0;
const NAV_GAP: f32 = 2.0;
const COLUMN_WIDTH: f32 = 720.0;
/// Pixels per wheel notch when the wheel reports lines.
const SCROLL_LINE: f32 = 48.0;
const SCROLLBAR_INSET: f32 = 6.0;
/// The settings page starts below the chrome; pointer y is window-relative.
const CHROME_OFFSET: f32 = crate::style::CHROME_HEIGHT;
pub(crate) const DISPLAY_FONT: &str = "Segoe UI Variable Display";

#[derive(Debug, Clone, Copy, Default, PartialEq, Eq)]
pub(crate) enum SettingsPane {
    #[default]
    General,
    Appearance,
    Privacy,
    Downloads,
    Extensions,
    Passwords,
}

struct PaneInfo {
    pane: SettingsPane,
    label: &'static str,
    glyph: &'static str,
    blurb: &'static str,
}

const PANES: &[PaneInfo] = &[
    PaneInfo {
        pane: SettingsPane::General,
        label: "General",
        glyph: icon::SETTINGS,
        blurb: "Startup, zoom, and how the browser engine runs.",
    },
    PaneInfo {
        pane: SettingsPane::Appearance,
        label: "Appearance",
        glyph: icon::PALETTE,
        blurb: "Themes, your own colours, and how the chrome moves.",
    },
    PaneInfo {
        pane: SettingsPane::Privacy,
        label: "Privacy",
        glyph: icon::SHIELD,
        blurb: "What gets blocked, what sites may ask for, and clearing data.",
    },
    PaneInfo {
        pane: SettingsPane::Downloads,
        label: "Downloads",
        glyph: icon::DOWNLOAD,
        blurb: "Where files go and what happens when they finish.",
    },
    PaneInfo {
        pane: SettingsPane::Extensions,
        label: "Extensions",
        glyph: icon::PUZZLE,
        blurb: "Add-ons that run inside the pages you visit.",
    },
    PaneInfo {
        pane: SettingsPane::Passwords,
        label: "Passwords",
        glyph: icon::KEY,
        blurb: "The encrypted vault that keeps your sign-ins.",
    },
];

const ZOOM_LEVELS: &[f32] = &[0.8, 0.9, 1.0, 1.1, 1.25, 1.5];

const TRACKING_LEVELS: &[(TrackingLevel, &str)] = &[
    (TrackingLevel::Off, "Off"),
    (TrackingLevel::Basic, "Basic"),
    (TrackingLevel::Balanced, "Balanced"),
    (TrackingLevel::Strict, "Strict"),
];

/// Search engines offered in settings, as `{query}` templates.
const SEARCH_ENGINES: &[(&str, &str)] = &[
    ("DuckDuckGo", "https://duckduckgo.com/?q={query}"),
    ("Brave", "https://search.brave.com/search?q={query}"),
    ("Google", "https://www.google.com/search?q={query}"),
    ("Bing", "https://www.bing.com/search?q={query}"),
];

const LOCK_MINUTES: &[u32] = &[5, 15, 30, 60];

impl Browser {
    pub(crate) fn open_settings(&mut self, cx: &mut Context<Self>) {
        if self.settings_open {
            return;
        }
        self.omnibox_editing = false;
        self.suggestions.clear();
        self.downloads_open = false;
        self.page_info_open = false;
        self.find_open = false;
        self.settings_open = true;
        self.prime_theme_fields(cx);
        // Replay the page's entrance every time the tab opens.
        self.springs.borrow_mut().replay("settings");
        self.springs.borrow_mut().replay("settings-pane");
        self.sync_views();
        // Esc and shortcuts must reach the browser root, not the hidden page.
        self.defer_in_window(cx, |b, window, cx| {
            crate::platform::take_focus(b.hwnd);
            window.focus(&b.focus, cx);
        });
        cx.notify();
    }

    pub(crate) fn close_settings(&mut self, cx: &mut Context<Self>) {
        if !self.settings_open {
            return;
        }
        self.settings_open = false;
        self.slider_drag = None;
        self.sync_views();
        self.focus_page();
        cx.notify();
    }

    /// The whole page area while the Settings tab is current.
    pub(crate) fn settings_page(&self, cx: &mut Context<Self>) -> Option<AnyElement> {
        let reveal = self.springs.borrow_mut().drive(
            "settings",
            0.0,
            f32::from(u8::from(self.settings_open)),
        );
        if !self.settings_open {
            return None;
        }
        let window_fill = crate::style::hsla(self.theme.colors.window);
        Some(
            div()
                .id("settings-page")
                .flex_1()
                .w_full()
                .min_h_0()
                .flex()
                .bg(window_fill)
                .text_size(px(13.0))
                .opacity(reveal)
                .child(self.settings_sidebar(cx))
                .child(self.settings_content(reveal, cx))
                .into_any_element(),
        )
    }

    /// The page area with a faded snapshot of the page and `card` on top.
    /// Used by the downloads and page-info sheets.
    pub(crate) fn sheet_backdrop(&self, card: AnyElement) -> AnyElement {
        div()
            .flex_1()
            .w_full()
            .relative()
            .children(
                self.preview
                    .clone()
                    .map(|image| img(image).absolute().size_full().opacity(0.3)),
            )
            .child(
                div()
                    .absolute()
                    .size_full()
                    .flex()
                    .justify_center()
                    .items_start()
                    .pt(px(16.0))
                    .child(card),
            )
            .into_any_element()
    }

    fn settings_sidebar(&self, cx: &mut Context<Self>) -> impl IntoElement {
        let c = self.colors;
        let selected = PANES
            .iter()
            .position(|p| p.pane == self.settings_pane)
            .unwrap_or(0);
        let at = self
            .springs
            .borrow_mut()
            .drive("settings-nav", selected as f32, selected as f32);
        div()
            .w(px(SIDEBAR_WIDTH))
            .flex_none()
            .h_full()
            .pt(px(28.0))
            .px(px(14.0))
            .flex()
            .flex_col()
            .gap(px(16.0))
            .bg(c.glass)
            .border_r_1()
            .border_color(c.border)
            .child(
                div()
                    .px(px(12.0))
                    .font_family(DISPLAY_FONT)
                    .text_size(px(20.0))
                    .font_weight(FontWeight::SEMIBOLD)
                    .child("Settings"),
            )
            .child(
                div()
                    .relative()
                    .flex()
                    .flex_col()
                    .gap(px(NAV_GAP))
                    .child(
                        div()
                            .absolute()
                            .top(px(at * (NAV_ROW + NAV_GAP)))
                            .left_0()
                            .right_0()
                            .h(px(NAV_ROW))
                            .rounded(px(self.theme.shape.radius_md))
                            .bg(c.selection),
                    )
                    .children(PANES.iter().map(|info| self.nav_item(info, cx))),
            )
    }

    /// One sidebar entry: an icon tile and a label. Hover washes in and
    /// nudges the label on the shared spring; the selection fill glides
    /// between entries underneath.
    fn nav_item(&self, info: &PaneInfo, cx: &mut Context<Self>) -> AnyElement {
        let c = self.colors;
        let (pane, active) = (info.pane, info.pane == self.settings_pane);
        let key = format!("nav-{}", info.label);
        let hover = self.hover_spring(&key);
        // Selection rides its own spring, so the tile fills with accent,
        // the glyph turns and the label brightens together as it moves.
        let sel = self.springs.borrow_mut().drive_keyed(
            format!("nav-sel-{}", info.label),
            f32::from(u8::from(active)),
            f32::from(u8::from(active)),
        );
        let idle = 1.0 - sel;
        let tile_rest = mix(c.field, c.hover, hover);
        let glyph_rest = c.text.opacity(0.55 + 0.45 * hover);
        let label = c.text.opacity(0.62 + 0.38 * hover.max(sel));
        div()
            .id(info.label)
            .h(px(NAV_ROW))
            .px(px(6.0))
            .flex()
            .items_center()
            .gap(px(10.0))
            .rounded(px(self.theme.shape.radius_md))
            .cursor_pointer()
            .bg(c.hover.opacity(hover * idle))
            .on_hover(self.hover_listener(key, cx))
            .active(|s| s.opacity(0.75))
            .on_click(cx.listener(move |this, _, _, cx| this.show_pane(pane, cx)))
            .child(
                div()
                    .size(px(26.0))
                    .flex_none()
                    .flex()
                    .items_center()
                    .justify_center()
                    .rounded(px(7.0))
                    .font_family(ICON_FONT)
                    .text_size(px(13.0))
                    .bg(mix(tile_rest, c.accent, sel))
                    .border_1()
                    .border_color(mix(c.border, c.accent, sel))
                    .text_color(mix(glyph_rest, c.on_accent, sel))
                    .child(info.glyph),
            )
            .child(
                div()
                    .ml(px(2.0 * hover * idle))
                    .text_color(label)
                    .when(active, |d| d.font_weight(FontWeight::MEDIUM))
                    .child(info.label),
            )
            .into_any_element()
    }

    pub(crate) fn show_pane(&mut self, pane: SettingsPane, cx: &mut Context<Self>) {
        if self.settings_pane == pane {
            return;
        }
        self.settings_pane = pane;
        self.theme_edit_role = None;
        self.settings_scroll = 0.0;
        self.springs.borrow_mut().snap("settings-scroll", 0.0);
        if pane == SettingsPane::Appearance {
            self.prime_theme_fields(cx);
        }
        self.springs.borrow_mut().replay("settings-pane");
        cx.notify();
    }

    fn settings_content(&self, reveal: f32, cx: &mut Context<Self>) -> impl IntoElement {
        let shown = self.springs.borrow_mut().drive("settings-pane", 0.0, 1.0);
        let info = PANES
            .iter()
            .find(|p| p.pane == self.settings_pane)
            .unwrap_or(&PANES[0]);
        let body: AnyElement = match self.settings_pane {
            SettingsPane::General => self.general_pane(cx),
            SettingsPane::Appearance => self.appearance_pane(cx),
            SettingsPane::Privacy => self.privacy_pane(cx),
            SettingsPane::Downloads => self.downloads_pane(cx),
            SettingsPane::Extensions => self.extensions_pane(cx),
            SettingsPane::Passwords => self.passwords_pane(cx),
        };
        let c = self.colors;
        // The column rises a little as the tab opens, and again (less) as
        // panes change; both settle on the shared spring.
        let rise = 16.0 * (1.0 - reveal) + 10.0 * (1.0 - shown);
        let scroll = self.settings_scroll_value();
        let (metrics_view, metrics_content) =
            (self.settings_metrics.clone(), self.settings_metrics.clone());
        let column = div()
            .absolute()
            .top(px(-scroll))
            .left_0()
            .w_full()
            .flex()
            .justify_center()
            .px(px(40.0))
            .pt(px(36.0 + rise))
            .pb(px(72.0))
            .child(
                canvas(
                    move |b, _, _| {
                        let (view, _) = metrics_content.get();
                        metrics_content.set((view, f32::from(b.size.height)));
                    },
                    |_, _, _, _| {},
                )
                .absolute()
                .size_full(),
            )
            .child(
                div()
                    .w_full()
                    .max_w(px(COLUMN_WIDTH))
                    .flex()
                    .flex_col()
                    .gap(px(28.0))
                    .opacity(shown)
                    .child(
                        div()
                            .flex()
                            .flex_col()
                            .gap(px(6.0))
                            .child(
                                div()
                                    .font_family(DISPLAY_FONT)
                                    .text_size(px(28.0))
                                    .font_weight(FontWeight::SEMIBOLD)
                                    .child(info.label),
                            )
                            .child(div().text_color(c.muted).child(info.blurb)),
                    )
                    .children(self.restart_banner(cx))
                    .child(body),
            );
        div()
            .id("settings-scroll")
            .relative()
            .flex_1()
            .min_w_0()
            .h_full()
            .overflow_hidden()
            .on_hover(self.hover_listener("settings-viewport".to_owned(), cx))
            .on_scroll_wheel(cx.listener(|this, e: &gpui::ScrollWheelEvent, _, cx| {
                let dy = f32::from(e.delta.pixel_delta(px(SCROLL_LINE)).y);
                this.scroll_settings_to(this.settings_scroll - dy, false, cx);
            }))
            .child(
                canvas(
                    move |b, _, _| {
                        let (_, content) = metrics_view.get();
                        metrics_view.set((f32::from(b.size.height), content));
                    },
                    |_, _, _, _| {},
                )
                .absolute()
                .size_full(),
            )
            .child(column)
            .children(self.settings_scrollbar(scroll, cx))
    }

    /// Where the column is drawn this frame. The spring chases the target
    /// unless smooth scrolling is off, then it jumps.
    fn settings_scroll_value(&self) -> f32 {
        let target = self.settings_scroll.clamp(0.0, self.settings_scroll_max());
        let mut springs = self.springs.borrow_mut();
        if !self.settings.engine.smooth_scrolling || self.settings_thumb_grab.is_some() {
            springs.snap("settings-scroll", target);
        }
        springs.drive("settings-scroll", target, target)
    }

    fn settings_scroll_max(&self) -> f32 {
        let (view, content) = self.settings_metrics.get();
        (content - view).max(0.0)
    }

    pub(crate) fn scroll_settings_to(&mut self, target: f32, snap: bool, cx: &mut Context<Self>) {
        self.settings_scroll = target.clamp(0.0, self.settings_scroll_max());
        if snap {
            self.springs
                .borrow_mut()
                .snap("settings-scroll", self.settings_scroll);
        }
        cx.notify();
    }

    /// A thin thumb on the right edge: faint at rest, fuller and wider while
    /// the pointer is over the page or the thumb is held. Drag it, or click
    /// the track to jump there.
    fn settings_scrollbar(&self, scroll: f32, cx: &mut Context<Self>) -> Option<AnyElement> {
        let (view, content) = self.settings_metrics.get();
        let max = (content - view).max(0.0);
        if max < 1.0 || view <= 0.0 {
            return None;
        }
        let c = self.colors;
        let held = self.settings_thumb_grab.is_some();
        let near = self.hover_spring("settings-viewport");
        let over = self
            .hover_spring("settings-thumb")
            .max(f32::from(u8::from(held)));
        let track = view - 2.0 * SCROLLBAR_INSET;
        let thumb = (track * view / content).clamp(36.0, track);
        let top = SCROLLBAR_INSET + (scroll / max) * (track - thumb);
        let width = 6.0 + 3.0 * over;
        Some(
            div()
                .id("settings-scrollbar")
                .absolute()
                .top_0()
                .right_0()
                .h_full()
                .w(px(16.0))
                .on_mouse_down(
                    MouseButton::Left,
                    cx.listener(move |this, e: &gpui::MouseDownEvent, _, cx| {
                        // A press on the bare track centres the thumb there.
                        let y = f32::from(e.position.y) - CHROME_OFFSET;
                        let t =
                            ((y - thumb / 2.0 - SCROLLBAR_INSET) / (track - thumb)).clamp(0.0, 1.0);
                        this.scroll_settings_to(t * max, false, cx);
                    }),
                )
                .child(
                    div()
                        .id("settings-thumb")
                        .absolute()
                        .top(px(top))
                        .right(px(4.0))
                        .w(px(width))
                        .h(px(thumb))
                        .rounded_full()
                        .bg(c.text.opacity(0.18 + 0.17 * near + 0.2 * over))
                        .cursor_pointer()
                        .on_hover(self.hover_listener("settings-thumb".to_owned(), cx))
                        .on_mouse_down(
                            MouseButton::Left,
                            cx.listener(move |this, e: &gpui::MouseDownEvent, _, cx| {
                                cx.stop_propagation();
                                let y = f32::from(e.position.y) - CHROME_OFFSET;
                                this.settings_thumb_grab = Some(y - top);
                                cx.notify();
                            }),
                        ),
                )
                .into_any_element(),
        )
    }

    /// Follows the pointer while the thumb is held.
    pub(crate) fn drag_settings_thumb(&mut self, pointer_y: f32, cx: &mut Context<Self>) {
        let Some(grab) = self.settings_thumb_grab else {
            return;
        };
        let (view, content) = self.settings_metrics.get();
        let max = (content - view).max(0.0);
        let track = view - 2.0 * SCROLLBAR_INSET;
        let thumb = (track * view / content.max(1.0)).clamp(36.0, track);
        let y = pointer_y - CHROME_OFFSET - grab - SCROLLBAR_INSET;
        let t = (y / (track - thumb).max(1.0)).clamp(0.0, 1.0);
        self.scroll_settings_to(t * max, true, cx);
    }

    /// Shown while browser-process switches differ from the running ones.
    fn restart_banner(&self, cx: &mut Context<Self>) -> Option<impl IntoElement> {
        if !self.restart_needed() {
            return None;
        }
        let c = self.colors;
        Some(
            div()
                .flex()
                .items_center()
                .gap(px(16.0))
                .px(px(16.0))
                .py(px(12.0))
                .rounded(px(self.theme.shape.radius_lg))
                .bg(c.selection)
                .child(div().flex_1().child(
                    "Restart Umbrella to apply browser engine changes. Your tabs come back.",
                ))
                .child(
                    self.action_button("restart", "Restart now", true, cx)
                        .on_click(cx.listener(|this, _, _, cx| this.restart(cx))),
                ),
        )
    }

    // ---- panes ---------------------------------------------------------

    fn general_pane(&self, cx: &mut Context<Self>) -> AnyElement {
        let s = &self.settings;
        let c = self.colors;
        let startup = usize::from(s.startup == Startup::StartPage);
        let zoom = ZOOM_LEVELS
            .iter()
            .position(|z| (*z - s.default_zoom).abs() < 0.001)
            .unwrap_or(2);
        let zoom_labels: Vec<String> = ZOOM_LEVELS
            .iter()
            .map(|z| format!("{}%", (z * 100.0).round()))
            .collect();
        let zoom_labels: Vec<&str> = zoom_labels.iter().map(String::as_str).collect();
        let folder = self.paths.root().display().to_string();
        stack()
            .child(self.group(
                "Startup",
                vec![
                    row(
                        "When Umbrella opens",
                        "Start page closes last session's tabs. Ctrl+Shift+T brings them back.",
                        c,
                        segmented(
                            &self.springs,
                            "startup",
                            &["Restore tabs", "Start page"],
                            startup,
                            c,
                            cx.listener(|this, index: &usize, _, cx| {
                                let startup = if *index == 0 {
                                    Startup::Restore
                                } else {
                                    Startup::StartPage
                                };
                                this.update_settings(cx, |s| s.startup = startup);
                            }),
                        ),
                    ),
                    row(
                        "Default zoom",
                        "For new tabs, and what Ctrl+0 resets to.",
                        c,
                        segmented(
                            &self.springs,
                            "zoom",
                            &zoom_labels,
                            zoom,
                            c,
                            cx.listener(|this, index: &usize, _, cx| {
                                let zoom = ZOOM_LEVELS[*index];
                                this.update_settings(cx, |s| s.default_zoom = zoom);
                            }),
                        ),
                    ),
                ],
            ))
            .child(self.group(
                "Performance",
                vec![
                    row(
                        "Hardware acceleration",
                        "Draw pages on the GPU. Turn off if pages flicker or go black. Needs a restart.",
                        c,
                        self.switch("gpu", s.engine.hardware_acceleration, cx, |s| {
                            s.engine.hardware_acceleration = !s.engine.hardware_acceleration
                        }),
                    ),
                    row(
                        "Smooth scrolling",
                        "Animate wheel and keyboard scrolling in pages. Needs a restart.",
                        c,
                        self.switch("smooth-scroll", s.engine.smooth_scrolling, cx, |s| {
                            s.engine.smooth_scrolling = !s.engine.smooth_scrolling
                        }),
                    ),
                    row(
                        "Fewer background processes",
                        "Runs audio inside the main process. One fewer entry in Task Manager, but an audio crash takes the browser down with it. Needs a restart.",
                        c,
                        self.switch("fewer-processes", s.engine.fewer_processes, cx, |s| {
                            s.engine.fewer_processes = !s.engine.fewer_processes
                        }),
                    ),
                    row(
                        "Autoplay with sound",
                        "When off, video and audio wait for a click before making sound. Needs a restart.",
                        c,
                        self.switch("autoplay", s.engine.autoplay_sound, cx, |s| {
                            s.engine.autoplay_sound = !s.engine.autoplay_sound
                        }),
                    ),
                ],
            ))
            .child(self.group("Import", vec![block(self.import_section(false, cx))]))
            .child(self.group(
                "Data",
                vec![row(
                    "Data folder",
                    &folder,
                    c,
                    self.action_button("open-folder", "Open", false, cx).on_click(
                        cx.listener(|this, _, _, cx| cx.open_with_system(this.paths.root())),
                    ),
                )],
            ))
            .into_any_element()
    }

    fn privacy_pane(&self, cx: &mut Context<Self>) -> AnyElement {
        let s = &self.settings;
        let c = self.colors;
        let engine = SEARCH_ENGINES
            .iter()
            .position(|(_, t)| *t == s.search_template)
            .unwrap_or(0);
        let names: Vec<&str> = SEARCH_ENGINES.iter().map(|(n, _)| *n).collect();
        let tracking = TRACKING_LEVELS
            .iter()
            .position(|(l, _)| *l == s.tracking_prevention)
            .unwrap_or(2);
        let tracking_names: Vec<&str> = TRACKING_LEVELS.iter().map(|(_, n)| *n).collect();
        let rules = self.filter_rule_count();
        let lists = if rules == 0 {
            "Filter lists are downloading or missing.".to_owned()
        } else {
            format!("{} rules from EasyList and EasyPrivacy.", thousands(rules))
        };
        stack()
            .child(self.group(
                "Blocking",
                vec![
                    row(
                        "Block ads and trackers",
                        &lists,
                        c,
                        div()
                            .flex()
                            .items_center()
                            .gap(px(10.0))
                            .child(self.action_button("update-lists", "Update now", false, cx).on_click(
                                cx.listener(|this, _, _, cx| this.load_filter_lists(true, cx)),
                            ))
                            .child(self.switch("adblock", s.block_ads, cx, |s| {
                                s.block_ads = !s.block_ads
                            })),
                    ),
                    row(
                        "Tracking prevention",
                        "Edge's built-in protection, on top of the filter lists. Strict can break some sign-ins.",
                        c,
                        segmented(
                            &self.springs,
                            "tracking",
                            &tracking_names,
                            tracking,
                            c,
                            cx.listener(|this, index: &usize, _, cx| {
                                let level = TRACKING_LEVELS[*index].0;
                                this.update_settings(cx, |s| s.tracking_prevention = level);
                            }),
                        ),
                    ),
                    row(
                        "Block pop-ups",
                        "Windows a page opens without a click. You can still open a blocked one.",
                        c,
                        self.switch("popups", s.block_popups, cx, |s| {
                            s.block_popups = !s.block_popups
                        }),
                    ),
                    row(
                        "Strip tracking parameters",
                        "Removes utm_, fbclid, gclid and similar from links before they load.",
                        c,
                        self.switch("strip-tracking", s.strip_tracking, cx, |s| {
                            s.strip_tracking = !s.strip_tracking
                        }),
                    ),
                ],
            ))
            .child(self.group(
                "Your IP address",
                vec![
                    row(
                        "WebRTC leak protection",
                        "Video calls use only your main connection, so they cannot reveal your local or real IP behind a VPN. Needs a restart.",
                        c,
                        self.switch("webrtc", s.engine.webrtc_protection, cx, |s| {
                            s.engine.webrtc_protection = !s.engine.webrtc_protection
                        }),
                    ),
                    row(
                        "Warn when a page looks up your IP",
                        "A notice in the toolbar when a page asks an IP-location service or opens WebRTC.",
                        c,
                        self.switch("warn-ip", s.warn_ip_lookups, cx, |s| {
                            s.warn_ip_lookups = !s.warn_ip_lookups
                        }),
                    ),
                ],
            ))
            .child(self.group(
                "Search",
                vec![row(
                    "Search engine",
                    "Used when the address bar text is not a web address.",
                    c,
                    segmented(
                        &self.springs,
                        "search",
                        &names,
                        engine,
                        c,
                        cx.listener(|this, index: &usize, _, cx| {
                            let template = SEARCH_ENGINES[*index].1.to_owned();
                            this.update_settings(cx, |s| s.search_template = template);
                        }),
                    ),
                )],
            ))
            .child(self.group(
                "Site permissions",
                vec![block(self.permission_list(cx))],
            ))
            .child(self.group(
                "Clear data",
                vec![
                    row(
                        "Browsing history",
                        "Removes every visited page from history and suggestions.",
                        c,
                        self.action_button("clear-history", "Clear", false, cx).on_click(cx.listener(
                            |this, _, _, cx| {
                                let _ = this.history.clear();
                                cx.notify();
                            },
                        )),
                    ),
                    row(
                        "Cookies and site data",
                        self.site_data_status(),
                        c,
                        self.action_button("clear-site-data", "Clear", false, cx)
                            .on_click(cx.listener(|this, _, _, cx| this.clear_site_data(cx))),
                    ),
                ],
            ))
            .into_any_element()
    }

    fn downloads_pane(&self, cx: &mut Context<Self>) -> AnyElement {
        let s = &self.settings.downloads;
        let c = self.colors;
        let folder = s
            .folder
            .as_ref()
            .map_or("Your Downloads folder".to_owned(), |f| {
                f.display().to_string()
            });
        let custom = s.folder.is_some();
        stack()
            .child(self.group(
                "Saving",
                vec![
                    row(
                        "Save files to",
                        &folder,
                        c,
                        div()
                            .flex()
                            .items_center()
                            .gap(px(8.0))
                            .when(custom, |d| {
                                d.child(self.action_button("reset-folder", "Reset", false, cx).on_click(
                                    cx.listener(|this, _, _, cx| {
                                        this.update_settings(cx, |s| s.downloads.folder = None)
                                    }),
                                ))
                            })
                            .child(self.action_button("change-folder", "Change…", false, cx).on_click(
                                cx.listener(|this, _, _, cx| this.choose_download_folder(cx)),
                            )),
                    ),
                    row(
                        "Ask where to save each file",
                        "Shows a save dialog before every download starts.",
                        c,
                        self.switch("ask-where", s.ask_where, cx, |s| {
                            s.downloads.ask_where = !s.downloads.ask_where
                        }),
                    ),
                    row(
                        "Open safe files when done",
                        "Documents, pictures, audio and video only. Programs and scripts never open on their own.",
                        c,
                        self.switch("open-when-done", s.open_when_done, cx, |s| {
                            s.downloads.open_when_done = !s.downloads.open_when_done
                        }),
                    ),
                ],
            ))
            .into_any_element()
    }

    fn choose_download_folder(&mut self, cx: &mut Context<Self>) {
        let picked = cx.prompt_for_paths(gpui::PathPromptOptions {
            files: false,
            directories: true,
            multiple: false,
            prompt: Some("Save downloads here".into()),
        });
        cx.spawn(async move |this, cx| {
            let Ok(Ok(Some(paths))) = picked.await else {
                return;
            };
            let Some(folder) = paths.into_iter().next() else {
                return;
            };
            let _ = this.update(cx, move |b, cx| {
                b.update_settings(cx, move |s| s.downloads.folder = Some(folder));
            });
        })
        .detach();
    }

    fn extensions_pane(&self, cx: &mut Context<Self>) -> AnyElement {
        stack()
            .child(self.group("Installed", vec![block(self.extensions_section(cx))]))
            .into_any_element()
    }

    fn passwords_pane(&self, cx: &mut Context<Self>) -> AnyElement {
        let s = &self.settings;
        let c = self.colors;
        let minutes = LOCK_MINUTES
            .iter()
            .position(|m| *m == s.vault_idle_lock_minutes)
            .unwrap_or(1);
        let labels: Vec<String> = LOCK_MINUTES.iter().map(|m| format!("{m} min")).collect();
        let labels: Vec<&str> = labels.iter().map(String::as_str).collect();
        stack()
            .child(self.group(
                "Vault",
                vec![
                    row(
                        "Unlock with Windows",
                        "Your Windows sign-in opens the vault, so this PC needs no master password.",
                        c,
                        toggle("dpapi", s.vault_dpapi, c, &self.springs).on_click(cx.listener(
                            |this, _, _, cx| {
                                this.update_settings(cx, |s| s.vault_dpapi = !s.vault_dpapi);
                                this.vault.set_dpapi(this.settings.vault_dpapi);
                            },
                        )),
                    ),
                    row(
                        "Lock when idle",
                        "The vault locks itself after this long without use.",
                        c,
                        segmented(
                            &self.springs,
                            "lock-minutes",
                            &labels,
                            minutes,
                            c,
                            cx.listener(|this, index: &usize, _, cx| {
                                let minutes = LOCK_MINUTES[*index];
                                this.update_settings(cx, |s| s.vault_idle_lock_minutes = minutes);
                            }),
                        ),
                    ),
                    row(
                        "Lock now",
                        "Forget the vault key until the next unlock.",
                        c,
                        self.action_button("lock-now", "Lock", false, cx).on_click(cx.listener(
                            |this, _, _, cx| {
                                this.lock_vault();
                                cx.notify();
                            },
                        )),
                    ),
                ],
            ))
            .child(self.generator_group(cx))
            .child(self.group("Import", vec![block(self.import_section(true, cx))]))
            .into_any_element()
    }

    fn generator_group(&self, cx: &mut Context<Self>) -> AnyElement {
        let s = &self.settings;
        let c = self.colors;
        let g = s.password_generator;
        let strength = format!(
            "{} characters, about {} bits of strength.",
            g.effective_length(),
            g.entropy_bits()
        );
        type Flip = fn(&mut GeneratorOptions);
        let classes: [(&'static str, &str, &str, bool, Flip); 4] = [
            ("gen-upper", "Uppercase letters", "A to Z", g.uppercase, |g| {
                g.uppercase = !g.uppercase
            }),
            ("gen-lower", "Lowercase letters", "a to z", g.lowercase, |g| {
                g.lowercase = !g.lowercase
            }),
            ("gen-digits", "Numbers", "0 to 9", g.digits, |g| g.digits = !g.digits),
            (
                "gen-symbols",
                "Symbols",
                "! @ # $ % ^ & * ( ) - _ = + [ ] { } ; : , . ? / ~",
                g.symbols,
                |g| g.symbols = !g.symbols,
            ),
        ];
        let last_on = g.class_count() == 1;
        let mut rows = vec![
            row(
                "Suggest strong passwords",
                "When you focus a new-password field, offer a generated one just below it.",
                c,
                self.switch("gen-suggest", s.suggest_passwords, cx, |s| {
                    s.suggest_passwords = !s.suggest_passwords
                }),
            ),
            row(
                "Length",
                &strength,
                c,
                self.length_stepper(g.effective_length(), cx),
            ),
        ];
        rows.extend(classes.into_iter().map(|(id, label, detail, on, flip)| {
            // The last class on stays on: a password needs something to draw from.
            let control = toggle(id, on, c, &self.springs);
            let control = if on && last_on {
                control.opacity(0.35).cursor_default()
            } else {
                control.on_click(cx.listener(move |this, _, _, cx| {
                    this.update_settings(cx, |s| flip(&mut s.password_generator));
                }))
            };
            row(label, detail, c, control)
        }));
        self.group("Password generator", rows)
    }

    /// Minus, the value, plus. Each end fades and stops at the allowed range.
    fn length_stepper(&self, length: usize, cx: &mut Context<Self>) -> impl IntoElement {
        let c = self.colors;
        let step = |id: &'static str, glyph: &'static str, delta: isize, enabled: bool| {
            div()
                .id(id)
                .size(px(26.0))
                .flex()
                .items_center()
                .justify_center()
                .rounded(px(6.0))
                .font_family(ICON_FONT)
                .text_size(px(10.0))
                .child(glyph)
                .when(!enabled, |d| d.opacity(0.35))
                .when(enabled, |d| {
                    d.cursor_pointer()
                        .hover(|s| s.bg(c.hover))
                        .active(|s| s.opacity(0.7))
                        .on_click(cx.listener(move |this, _, _, cx| {
                            this.update_settings(cx, |s| {
                                let g = &mut s.password_generator;
                                g.length = g
                                    .effective_length()
                                    .saturating_add_signed(delta)
                                    .clamp(generator::MIN_LENGTH, generator::MAX_LENGTH);
                            });
                        }))
                })
        };
        let shorter = step("gen-shorter", icon::REMOVE, -1, length > generator::MIN_LENGTH);
        let longer = step("gen-longer", icon::ADD, 1, length < generator::MAX_LENGTH);
        div()
            .flex()
            .items_center()
            .p(px(3.0))
            .rounded(px(8.0))
            .bg(c.field)
            .border_1()
            .border_color(c.border)
            .child(shorter)
            .child(
                div()
                    .w(px(36.0))
                    .flex()
                    .justify_center()
                    .text_size(px(13.0))
                    .font_weight(FontWeight::MEDIUM)
                    .child(length.to_string()),
            )
            .child(longer)
    }

    // ---- building blocks ----------------------------------------------

    /// A titled group: one rounded surface, rows split by hairlines.
    pub(crate) fn group(&self, title: &'static str, rows: Vec<AnyElement>) -> AnyElement {
        let c = self.colors;
        div()
            .flex()
            .flex_col()
            .gap(px(10.0))
            .child(
                div()
                    .pl(px(4.0))
                    .font_weight(FontWeight::SEMIBOLD)
                    .child(title),
            )
            .child(
                div()
                    .flex()
                    .flex_col()
                    .rounded(px(self.theme.shape.radius_lg))
                    .bg(c.field)
                    .border_1()
                    .border_color(c.border)
                    .overflow_hidden()
                    .children(rows.into_iter().enumerate().map(move |(i, row)| {
                        div()
                            .when(i > 0, |d| d.border_t_1().border_color(c.border))
                            .child(row)
                    })),
            )
            .into_any_element()
    }

    /// The settings button: a quiet field-filled one, or the single accent
    /// one a group may carry. Hover and press ride the shared spring.
    pub(crate) fn action_button(
        &self,
        id: impl Into<gpui::ElementId>,
        label: impl Into<SharedString>,
        primary: bool,
        cx: &mut Context<Self>,
    ) -> gpui::Stateful<gpui::Div> {
        let c = self.colors;
        let id: gpui::ElementId = id.into();
        let key = format!("button-{id}");
        let hover = self.hover_spring(&key);
        let (fill, edge, text) = if primary {
            (
                mix(c.accent, c.on_accent, 0.14 * hover),
                mix(c.accent, c.on_accent, 0.14 * hover),
                c.on_accent,
            )
        } else {
            (
                mix(c.field, c.text.opacity(0.14), hover),
                mix(c.border, c.text.opacity(0.3), hover),
                c.text,
            )
        };
        div()
            .id(id)
            .flex_none()
            .px(px(if primary { 14.0 } else { 12.0 }))
            .h(px(28.0))
            .flex()
            .items_center()
            .rounded(px(7.0))
            .bg(fill)
            .border_1()
            .border_color(edge)
            .text_color(text)
            .text_size(px(12.0))
            .font_weight(if primary {
                FontWeight::SEMIBOLD
            } else {
                FontWeight::MEDIUM
            })
            .cursor_pointer()
            .on_hover(self.hover_listener(key, cx))
            .active(|s| s.opacity(0.75))
            .child(label.into())
    }

    /// A switch bound to one boolean setting.
    fn switch(
        &self,
        id: &'static str,
        on: bool,
        cx: &mut Context<Self>,
        flip: impl Fn(&mut browser_core::settings::Settings) + 'static,
    ) -> gpui::Stateful<gpui::Div> {
        let flip = std::rc::Rc::new(flip);
        toggle(id, on, self.colors, &self.springs).on_click(cx.listener(move |this, _, _, cx| {
            let flip = flip.clone();
            this.update_settings(cx, move |s| flip(s));
        }))
    }

    pub(crate) fn current_theme_id(&self) -> ThemeId {
        self.settings
            .theme
            .clone()
            .filter(|id| self.themes.get(id).is_some())
            .unwrap_or_else(browser_core::theme::presets::default_id)
    }
}

fn stack() -> gpui::Div {
    div().flex().flex_col().gap(px(28.0))
}

/// A row of a group: what it is on the left, its control on the right.
pub(crate) fn row(
    label: &str,
    detail: &str,
    c: crate::style::ChromeColors,
    control: impl IntoElement,
) -> AnyElement {
    let (label, detail): (SharedString, SharedString) =
        (label.to_owned().into(), detail.to_owned().into());
    div()
        .min_h(px(60.0))
        .px(px(16.0))
        .py(px(12.0))
        .flex()
        .items_center()
        .gap(px(20.0))
        .child(
            div()
                .flex_1()
                .min_w_0()
                .flex()
                .flex_col()
                .gap(px(3.0))
                .child(div().child(label))
                .child(
                    div()
                        .text_size(px(12.0))
                        .line_height(px(17.0))
                        .text_color(c.muted)
                        .child(detail),
                ),
        )
        .child(div().flex_none().child(control))
        .into_any_element()
}

/// Free-form content inside a group, padded like a row.
pub(crate) fn block(content: impl IntoElement) -> AnyElement {
    div()
        .px(px(16.0))
        .py(px(14.0))
        .child(content)
        .into_any_element()
}

pub(crate) fn toggle_for(
    id: impl Into<gpui::ElementId>,
    on: bool,
    c: crate::style::ChromeColors,
    springs: &std::cell::RefCell<crate::motion::Springs>,
) -> gpui::Stateful<gpui::Div> {
    toggle(id, on, c, springs)
}

/// A switch whose knob slides and whose fill fades on the shared spring.
fn toggle(
    id: impl Into<gpui::ElementId>,
    on: bool,
    c: crate::style::ChromeColors,
    springs: &std::cell::RefCell<crate::motion::Springs>,
) -> gpui::Stateful<gpui::Div> {
    let id: gpui::ElementId = id.into();
    let target = f32::from(u8::from(on));
    let v = springs
        .borrow_mut()
        .drive_keyed(format!("toggle-{id}"), target, target);
    div()
        .id(id)
        .relative()
        .flex_none()
        .w(px(40.0))
        .h(px(22.0))
        .rounded_full()
        .bg(c.field)
        .border_1()
        .border_color(c.border)
        .cursor_pointer()
        .child(
            div()
                .absolute()
                .size_full()
                .rounded_full()
                .bg(c.accent)
                .opacity(v),
        )
        .child(
            div()
                .absolute()
                .top(px(3.0))
                .left(px(3.0 + 18.0 * v))
                .size(px(14.0))
                .rounded_full()
                .shadow_sm()
                .bg(if v > 0.5 { c.on_accent } else { c.muted }),
        )
}

/// Equal-width segments with one highlight that glides to the choice.
pub(crate) fn segmented(
    springs: &std::cell::RefCell<crate::motion::Springs>,
    id: &'static str,
    options: &[&str],
    selected: usize,
    c: crate::style::ChromeColors,
    on_select: impl Fn(&usize, &mut Window, &mut gpui::App) + 'static,
) -> impl IntoElement {
    let on_select = std::rc::Rc::new(on_select);
    let longest = options.iter().map(|o| o.chars().count()).max().unwrap_or(4) as f32;
    let width = (longest * 7.0 + 22.0).max(50.0);
    let at =
        springs
            .borrow_mut()
            .drive_keyed(format!("segment-{id}"), selected as f32, selected as f32);
    div()
        .relative()
        .flex()
        .p(px(3.0))
        .rounded(px(8.0))
        .bg(c.field)
        .border_1()
        .border_color(c.border)
        .child(
            div()
                .absolute()
                .top(px(3.0))
                .left(px(3.0 + at * width))
                .w(px(width))
                .h(px(26.0))
                .rounded(px(6.0))
                .bg(c.capsule)
                .shadow_sm(),
        )
        .children(options.iter().enumerate().map(|(i, label)| {
            let on_select = on_select.clone();
            let label: SharedString = (*label).to_owned().into();
            let active = i == selected;
            div()
                .id((id, i))
                .w(px(width))
                .h(px(26.0))
                .flex()
                .items_center()
                .justify_center()
                .rounded(px(6.0))
                .text_size(px(12.0))
                .cursor_pointer()
                .when(active, |d| d.font_weight(FontWeight::MEDIUM))
                .when(!active, |d| {
                    d.text_color(c.muted).hover(|s| s.opacity(0.85))
                })
                .active(|s| s.opacity(0.7))
                .on_click(move |_, window, cx| on_select(&i, window, cx))
                .child(label)
        }))
}

/// 134655 -> "134,655".
fn thousands(n: usize) -> String {
    let digits = n.to_string();
    let mut out = String::with_capacity(digits.len() + digits.len() / 3);
    for (i, ch) in digits.chars().enumerate() {
        if i > 0 && (digits.len() - i).is_multiple_of(3) {
            out.push(',');
        }
        out.push(ch);
    }
    out
}

#[cfg(test)]
mod tests {
    #[test]
    fn groups_thousands() {
        assert_eq!(super::thousands(134_655), "134,655");
        assert_eq!(super::thousands(999), "999");
        assert_eq!(super::thousands(1_000_000), "1,000,000");
    }
}
