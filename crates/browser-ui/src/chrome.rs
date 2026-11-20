//! The window's chrome: a tab row that is also the title bar, and a toolbar
//! with the omnibox. Everything below `CHROME_HEIGHT` belongs to WebView2.

use std::time::{Duration, Instant};

use browser_core::SavedTab;
use browser_core::TabId;
use browser_engine::LogicalRect;
use gpui::{
    Animation, AnimationExt, AnyElement, BoxShadow, ColorExt, Context, FontWeight, Hsla,
    InteractiveElement, IntoElement, MouseButton, ParentElement, Render, SharedString, Styled,
    Window, WindowControlArea, div, img, prelude::*, px,
};
use gpui_ce_elements::editable_text::actions::{Enter, Escape, NavDown, NavUp};
use gpui_ce_elements::editable_text::text_input;
use url::Url;

use crate::actions;
use crate::browser::{Browser, NEW_TAB_URL};
use crate::motion::{Slot, layout_tabs};
use crate::style::{CAPTION_BUTTON, CHROME_HEIGHT, ICON_FONT, TAB_ROW, TOOLBAR_ROW, UI_FONT, icon};
use crate::suggest::{Kind, Suggestion};
use crate::vault_ui::Prompt;

const STRIP_START: f32 = 8.0;
const OMNIBOX_GROW: f32 = 48.0;
/// How far panels travel as they appear.
pub(crate) const RISE: f32 = 12.0;
const SCANLINE_PITCH: f32 = 3.0;
/// Room for the workspace name pill, shown once there is more than one.
const WORKSPACE_PILL: f32 = 116.0;
const TAB_GAP: f32 = 2.0;
const NEW_TAB_BUTTON: f32 = 32.0;
const TAB_HEIGHT: f32 = 28.0;

impl Render for Browser {
    fn render(&mut self, window: &mut Window, cx: &mut Context<Self>) -> impl IntoElement {
        self.place_content(window);
        let layout = self.tab_layout(window);
        self.tab_width = layout.first().map_or(0.0, |(_, s)| s.width);
        let active = self.strip_active();
        self.motion.sync(&layout, active);
        self.remember_tab_titles();
        self.advance_motion(window);

        let colors = self.colors;
        let dim = !window.is_window_active();
        let slab = (!self.page_fullscreen).then(|| self.chrome_slab(window, dim, cx));
        let root = div()
            .id("browser")
            .key_context("Browser")
            .track_focus(&self.focus);
        let root = actions::route(root, cx)
            .size_full()
            .flex()
            .flex_col()
            .font_family(UI_FONT)
            .text_color(colors.text)
            .on_modifiers_changed(cx.listener(|this, e: &gpui::ModifiersChangedEvent, _, cx| {
                if !e.modifiers.control {
                    this.commit_cycle();
                    cx.notify();
                }
            }))
            .on_action(cx.listener(|this, _: &Enter, window, cx| this.submit_omnibox(window, cx)))
            .on_action(cx.listener(|this, _: &Escape, _, cx| {
                if this.downloads_open {
                    return this.close_downloads(cx);
                }
                if this.page_info_open {
                    return this.close_page_info(cx);
                }
                this.close_suggestions(cx);
                this.focus_page();
                cx.notify();
            }))
            .children(slab)
            .children(self.suggestion_panel(cx))
            .children(self.settings_page(cx))
            .children(self.downloads_sheet(window, cx))
            .children(self.page_info_sheet(window, cx))
            .children(self.start_page(cx))
            .children(self.split_gutter(window, cx))
            .on_mouse_move(cx.listener(|this, e: &gpui::MouseMoveEvent, window, cx| {
                if this.settings_thumb_grab.is_some() {
                    if e.pressed_button == Some(MouseButton::Left) {
                        this.drag_settings_thumb(f32::from(e.position.y), cx);
                    } else {
                        this.settings_thumb_grab = None;
                        cx.notify();
                    }
                    return;
                }
                // A theme slider keeps following the pointer anywhere in the window.
                if let Some(kind) = this.slider_drag {
                    if e.pressed_button == Some(MouseButton::Left) {
                        this.slide(kind, e.position, window, cx);
                    } else {
                        this.end_slide(window, cx);
                    }
                    return;
                }
                if !this.split_dragging {
                    return;
                }
                if e.pressed_button == Some(MouseButton::Left) {
                    this.drag_split_to(f32::from(e.position.x));
                } else {
                    this.split_dragging = false;
                }
                cx.notify();
            }))
            .on_mouse_up(
                MouseButton::Left,
                cx.listener(|this, _, window, cx| {
                    this.split_dragging = false;
                    this.settings_thumb_grab = None;
                    this.end_slide(window, cx);
                    cx.notify();
                }),
            );
        self.keep_animating(window);
        root
    }
}

impl Browser {
    /// Lays out the page area and records where the omnibox sits.
    fn place_content(&mut self, window: &mut Window) {
        let size = window.viewport_size();
        let width = f32::from(size.width);
        let top = self.chrome_height();
        let rect = LogicalRect {
            x: 0.0,
            y: top,
            width,
            height: (f32::from(size.height) - top).max(0.0),
        };
        self.set_content_bounds(rect, window.scale_factor());
        let focus = self.springs.borrow_mut().drive(
            "omnibox",
            0.0,
            f32::from(u8::from(self.omnibox_editing)),
        );
        // Safari-style: the field widens a little while you type in it.
        let omnibox = ((width * 0.5).clamp(280.0_f32.min(width), 720.0) + OMNIBOX_GROW * focus)
            .min(self.omnibox_room(width));
        self.omnibox_span = ((width - omnibox) / 2.0, omnibox);
    }

    /// Widest centred omnibox that clears both toolbar clusters, so the
    /// vault and download pills never sit under it.
    fn omnibox_room(&self, width: f32) -> f32 {
        let left = 8.0 + 3.0 * 30.0 + 4.0 + 8.0;
        let mut right = 8.0 + 2.0 * 30.0 + 50.0 + 8.0;
        if self.downloads.latest().is_some() {
            right += 184.0;
        }
        if self.vault.prompt.is_some() || self.active_permission_prompt().is_some() {
            right += 360.0;
        }
        (width - 2.0 * f32::max(left, right) - 16.0).max(200.0)
    }

    fn tab_layout(&self, window: &Window) -> Vec<(TabId, Slot)> {
        let width = f32::from(window.viewport_size().width);
        let start = self.strip_start();
        let available = width - start - NEW_TAB_BUTTON - CAPTION_BUTTON * 3.0 - 48.0;
        let mut ids: Vec<TabId> = self
            .workspace()
            .map(|w| w.tabs.iter().map(|t| t.id).collect())
            .unwrap_or_default();
        if self.settings_open {
            ids.push(crate::browser::SETTINGS_TAB);
        }
        layout_tabs(&ids, start, available.max(0.0), TAB_GAP)
    }

    /// Springs only request frames while moving, so an idle window is free.
    /// Panels pick their spring targets while the tree is built, after
    /// `advance_motion` ran, so a spring that just got a new target asks for
    /// its first frame here.
    fn keep_animating(&mut self, window: &mut Window) {
        if self.last_frame.is_none() && self.springs.borrow().moving() {
            self.last_frame = Some(Instant::now());
            window.request_animation_frame();
        }
    }

    fn advance_motion(&mut self, window: &mut Window) {
        let now = Instant::now();
        let dt = self
            .last_frame
            .map_or(0.0, |t| now.duration_since(t).as_secs_f32());
        let tabs_moving = self.motion.step(dt, self.reduced_motion);
        self.springs.borrow_mut().step(dt, self.reduced_motion);
        if tabs_moving || self.springs.borrow().moving() {
            self.last_frame = Some(now);
            window.request_animation_frame();
        } else {
            self.last_frame = None;
        }
    }

    // ---- tab row -------------------------------------------------------

    fn tab_row(&self, window: &Window, dim: bool, cx: &mut Context<Self>) -> impl IntoElement {
        let tabs: Vec<SavedTab> = self.workspace().map(|w| w.tabs.clone()).unwrap_or_default();
        let active = self.strip_active();
        let last = if self.settings_open {
            Some(crate::browser::SETTINGS_TAB)
        } else {
            tabs.last().map(|t| t.id)
        };
        let end = last
            .and_then(|id| self.motion.slot(id))
            .map_or(self.strip_start(), |s| s.x + s.width);
        let capsule = self.dragged_slot(active, self.motion.capsule());
        div()
            .h(px(TAB_ROW))
            .w_full()
            .flex()
            .child(
                div()
                    .id("tab-strip")
                    .relative()
                    .flex_1()
                    .h_full()
                    .on_mouse_move(cx.listener(|this, e: &gpui::MouseMoveEvent, _, cx| {
                        if this.tab_drag.is_none() {
                            return;
                        }
                        if e.pressed_button != Some(MouseButton::Left) {
                            this.end_tab_drag();
                        } else {
                            let start = this.strip_start();
                            this.drag_tab_to(f32::from(e.position.x), start, TAB_GAP);
                        }
                        cx.notify();
                    }))
                    .on_mouse_up(
                        MouseButton::Left,
                        cx.listener(|this, _, _, cx| {
                            this.end_tab_drag();
                            cx.notify();
                        }),
                    )
                    .children(self.workspace_pill(cx))
                    .children(self.ghost_tabs())
                    .child(self.capsule(capsule))
                    .children(tabs.iter().filter_map(|t| {
                        let slot = self.dragged_slot(Some(t.id), self.motion.slot(t.id)?);
                        Some(self.tab(t, slot, Some(t.id) == active, dim, cx))
                    }))
                    .children(self.settings_tab(dim, cx))
                    .child(self.new_tab_button(end, cx))
                    .child(drag_region(end + NEW_TAB_BUTTON + 8.0)),
            )
            .child(self.caption_buttons(window))
    }

    /// The draggable gap between split panes. GPUI owns it because no page
    /// covers it, and it captures the mouse on press, so the drag keeps
    /// working when the pointer crosses into a page.
    fn split_gutter(&self, window: &Window, cx: &mut Context<Self>) -> Option<impl IntoElement> {
        if self.suggestions_open() {
            return None;
        }
        let x = self.split_gutter_x()?;
        let height = (f32::from(window.viewport_size().height) - CHROME_HEIGHT).max(0.0);
        let colors = self.colors;
        let grip = if self.split_dragging {
            colors.accent
        } else {
            colors.border
        };
        Some(
            div()
                .id("split-gutter")
                .absolute()
                .left(px(x))
                .top(px(CHROME_HEIGHT))
                .w(px(crate::browser::SPLIT_GUTTER))
                .h(px(height))
                .flex()
                .justify_center()
                .items_center()
                .cursor_col_resize()
                .hover(move |s| s.bg(colors.hover))
                .on_mouse_down(
                    MouseButton::Left,
                    cx.listener(|this, _, _, cx| {
                        this.split_dragging = true;
                        cx.notify();
                    }),
                )
                .child(div().w(px(2.0)).h(px(36.0)).rounded(px(1.0)).bg(grip)),
        )
    }

    /// Tab row and toolbar on one frosted slab.
    fn chrome_slab(&self, window: &Window, dim: bool, cx: &mut Context<Self>) -> AnyElement {
        let colors = self.colors;
        div()
            .relative()
            .h(px(self.chrome_height()))
            .w_full()
            .flex()
            .flex_col()
            .bg(colors.glass)
            .border_b_1()
            .border_color(colors.border)
            .on_mouse_down(
                MouseButton::Left,
                cx.listener(|this, e: &gpui::MouseDownEvent, window, cx| {
                    this.press_outside_omnibox(e.position, window, cx)
                }),
            )
            // Inactive windows quiet their chrome, as native captions do.
            .when(dim, |d| d.text_color(colors.muted))
            .child(self.tab_row(window, dim, cx))
            .child(self.toolbar(window, cx))
            .children(self.find_bar(cx))
            .children(self.tab_hover_card())
            .children(self.scanlines())
            .into_any_element()
    }

    /// No chrome while a page video is fullscreen.
    fn chrome_height(&self) -> f32 {
        if self.page_fullscreen {
            0.0
        } else if self.find_open {
            CHROME_HEIGHT + crate::find_bar::FIND_ROW
        } else {
            CHROME_HEIGHT
        }
    }

    fn strip_start(&self) -> f32 {
        if self.has_many_workspaces() {
            STRIP_START + WORKSPACE_PILL
        } else {
            STRIP_START
        }
    }

    /// The active workspace's name; clicking it lists the others.
    fn workspace_pill(&self, cx: &mut Context<Self>) -> Option<impl IntoElement> {
        if !self.has_many_workspaces() {
            return None;
        }
        let name: SharedString = self
            .workspace()
            .map(|w| w.name.clone())
            .unwrap_or_default()
            .into();
        let colors = self.colors;
        Some(
            div()
                .id("workspace")
                .absolute()
                .left(px(STRIP_START))
                .top(px((TAB_ROW - TAB_HEIGHT) / 2.0 + 2.0))
                .w(px(WORKSPACE_PILL - 8.0))
                .h(px(TAB_HEIGHT))
                .flex()
                .items_center()
                .gap(px(6.0))
                .px(px(10.0))
                .rounded(px(self.theme.shape.radius_md))
                .border_1()
                .border_color(colors.border)
                .text_size(px(12.0))
                .font_weight(FontWeight::MEDIUM)
                .hover(move |s| s.bg(colors.hover))
                .on_click(cx.listener(|this, _, window, cx| {
                    this.open_palette(window, cx);
                    this.omnibox
                        .update(cx, |field, cx| field.emplace(">workspace", cx));
                }))
                .child(glyph_text(icon::WORKSPACE, 11.0).text_color(colors.muted))
                .child(div().min_w_0().truncate().child(name)),
        )
    }

    /// Closing tabs keep their title so they can shrink away in place.
    fn remember_tab_titles(&mut self) {
        let titles: Vec<(TabId, String)> = self
            .workspace()
            .map(|w| w.tabs.iter().map(|t| (t.id, tab_title(t))).collect())
            .unwrap_or_default();
        for (id, title) in titles {
            self.motion.remember_title(id, &title);
        }
        self.motion
            .remember_title(crate::browser::SETTINGS_TAB, "Settings");
    }

    fn ghost_tabs(&self) -> Vec<AnyElement> {
        let c = self.colors;
        self.motion
            .ghosts()
            .iter()
            .map(|ghost| {
                let title: SharedString = ghost.title.clone().into();
                div()
                    .absolute()
                    .left(px(ghost.x))
                    .top(px((TAB_ROW - TAB_HEIGHT) / 2.0 + 2.0))
                    .w(px(ghost.width()))
                    .h(px(TAB_HEIGHT))
                    .overflow_hidden()
                    .opacity(ghost.presence())
                    .flex()
                    .items_center()
                    .pl(px(10.0))
                    .text_size(px(12.0))
                    .text_color(c.muted)
                    .child(div().truncate().child(title))
                    .into_any_element()
            })
            .collect()
    }

    fn capsule(&self, slot: Slot) -> impl IntoElement {
        div()
            .absolute()
            .left(px(slot.x))
            .top(px((TAB_ROW - TAB_HEIGHT) / 2.0 + 2.0))
            .w(px(slot.width))
            .h(px(TAB_HEIGHT))
            .rounded(px(self.theme.shape.radius_md))
            .bg(self.colors.capsule)
            .border_1()
            .border_color(self.colors.capsule_edge)
            .shadow(self.capsule_shadow())
    }

    /// A phosphor bloom around the active tab for themes with `glow`; the
    /// usual soft drop shadow otherwise.
    fn capsule_shadow(&self) -> Vec<BoxShadow> {
        let glow = self.theme.effects.glow;
        if glow <= 0.0 {
            return vec![
                BoxShadow::new(px(0.0), px(1.0), gpui::black().opacity(0.12)).blur_radius(px(2.0)),
            ];
        }
        let color = crate::style::hsla(self.theme.colors.accent.with_alpha(glow * 0.8));
        vec![BoxShadow::new(px(0.0), px(0.0), color).blur_radius(px(4.0 + 10.0 * glow))]
    }

    /// Horizontal scan lines over the chrome for themes with `scanlines`.
    /// They paint only on the chrome; pages are never drawn over.
    fn scanlines(&self) -> Option<impl IntoElement> {
        let strength = self.theme.effects.scanlines;
        if strength <= 0.0 {
            return None;
        }
        let line = gpui::black().opacity(strength * 0.45);
        let rows = (CHROME_HEIGHT / SCANLINE_PITCH) as usize;
        Some(
            div()
                .absolute()
                .top_0()
                .left_0()
                .size_full()
                .children((0..rows).map(move |i| {
                    div()
                        .absolute()
                        .left_0()
                        .w_full()
                        .top(px(i as f32 * SCANLINE_PITCH))
                        .h(px(1.0))
                        .bg(line)
                })),
        )
    }

    /// The dragged tab (and its capsule) follows the pointer, not its spring.
    fn dragged_slot(&self, id: Option<TabId>, slot: Slot) -> Slot {
        match self.tab_drag {
            Some(drag) if drag.moved && Some(drag.id) == id => Slot {
                x: drag.pointer - drag.grab,
                ..slot
            },
            _ => slot,
        }
    }

    fn tab(
        &self,
        tab: &SavedTab,
        slot: Slot,
        active: bool,
        dim: bool,
        cx: &mut Context<Self>,
    ) -> AnyElement {
        let id = tab.id;
        let colors = self.colors;
        let title: SharedString = tab_title(tab).into();
        div()
            .id(("tab", id.0))
            .group("tab")
            .absolute()
            .left(px(slot.x))
            .top(px((TAB_ROW - TAB_HEIGHT) / 2.0 + 2.0))
            .w(px(slot.width))
            .h(px(TAB_HEIGHT))
            .flex()
            .items_center()
            .gap(px(6.0))
            .pl(px(10.0))
            .pr(px(4.0))
            .rounded(px(self.theme.shape.radius_md))
            .text_size(px(12.0))
            .text_color(if active && !dim {
                colors.text
            } else {
                colors.muted
            })
            .when(!active, |d| d.hover(|s| s.bg(colors.hover)))
            .on_hover(cx.listener(move |this, over: &bool, _, cx| {
                this.on_tab_hover(id, *over);
                cx.notify();
            }))
            .on_mouse_down(
                MouseButton::Left,
                cx.listener(move |this, e: &gpui::MouseDownEvent, _, cx| {
                    this.activate_tab(id);
                    this.begin_tab_drag(id, f32::from(e.position.x), slot.x);
                    cx.notify();
                }),
            )
            .on_mouse_down(
                MouseButton::Middle,
                cx.listener(move |this, _, _, cx| {
                    this.close_tab(id);
                    cx.notify();
                }),
            )
            .child(self.tab_glyph(tab, cx))
            .child(div().flex_1().min_w_0().truncate().child(title))
            .child(self.tab_close(id, active, cx))
            .into_any_element()
    }

    /// Settings as a tab of its own: gear glyph, title and close button.
    fn settings_tab(&self, dim: bool, cx: &mut Context<Self>) -> Option<AnyElement> {
        if !self.settings_open {
            return None;
        }
        let slot = self.motion.slot(crate::browser::SETTINGS_TAB)?;
        let colors = self.colors;
        Some(
            div()
                .id("settings-tab")
                .absolute()
                .left(px(slot.x))
                .top(px((TAB_ROW - TAB_HEIGHT) / 2.0 + 2.0))
                .w(px(slot.width))
                .h(px(TAB_HEIGHT))
                .flex()
                .items_center()
                .gap(px(6.0))
                .pl(px(10.0))
                .pr(px(4.0))
                .text_size(px(12.0))
                .text_color(if dim { colors.muted } else { colors.text })
                .child(glyph_text(icon::SETTINGS, 11.0))
                .child(div().flex_1().min_w_0().truncate().child("Settings"))
                .child(
                    self.icon_button("settings-tab-close", icon::CLOSE, 20.0, 8.0, cx)
                        .on_click(cx.listener(|this, _, _, cx| this.close_settings(cx))),
                )
                .into_any_element(),
        )
    }

    fn tab_glyph(&self, tab: &SavedTab, cx: &mut Context<Self>) -> AnyElement {
        let id = tab.id;
        let audible = self.is_playing(id) || tab.muted;
        if !audible {
            return self.page_glyph(id);
        }
        let glyph = if tab.muted { icon::MUTE } else { icon::VOLUME };
        self.icon_button(("mute", id.0), glyph, 22.0, 11.0, cx)
            .on_click(cx.listener(move |this, _, _, cx| {
                this.toggle_mute(id);
                cx.notify();
            }))
            .into_any_element()
    }

    /// The page glyph breathes while loading, distinct from the reload icon;
    /// with reduced motion it is a dimmed static glyph instead.
    fn page_glyph(&self, id: TabId) -> AnyElement {
        let url = self
            .saved_tab(id)
            .map(|t| t.url.clone())
            .unwrap_or_default();
        let glyph = match self.icon_for(&url) {
            Some(image) => div()
                .size(px(14.0))
                .flex_none()
                .child(img(image).size_full()),
            None => glyph_text(icon::GLOBE, 11.0),
        };
        if !self.is_loading(id) {
            return glyph.into_any_element();
        }
        if self.reduced_motion {
            return glyph.opacity(0.5).into_any_element();
        }
        glyph
            .with_animation(
                ("loading", id.0),
                Animation::new(Duration::from_millis(1100)).repeat(),
                |el, t| el.opacity(0.3 + 0.7 * (1.0 - (t * 2.0 - 1.0).abs())),
            )
            .into_any_element()
    }

    fn tab_close(&self, id: TabId, active: bool, cx: &mut Context<Self>) -> impl IntoElement {
        self.icon_button(("close", id.0), icon::CLOSE, 20.0, 8.0, cx)
            .when(!active, |d| {
                d.invisible().group_hover("tab", |s| s.visible())
            })
            .on_click(cx.listener(move |this, _, _, cx| {
                this.close_tab(id);
                cx.notify();
            }))
    }

    fn new_tab_button(&self, x: f32, cx: &mut Context<Self>) -> impl IntoElement {
        self.icon_button("new-tab", icon::ADD, 28.0, 11.0, cx)
            .absolute()
            .left(px(x + 4.0))
            .top(px((TAB_ROW - 28.0) / 2.0 + 2.0))
            .text_color(self.colors.muted)
            .on_click(cx.listener(|this, _, window, cx| {
                this.open_tab(NEW_TAB_URL, None, true);
                this.focus_omnibox(window, cx);
            }))
    }

    fn caption_buttons(&self, window: &Window) -> impl IntoElement {
        let colors = self.colors;
        let restore = if window.is_maximized() {
            icon::RESTORE
        } else {
            icon::MAXIMIZE
        };
        let caption =
            |id: &'static str, glyph: &'static str, area: WindowControlArea, hover: Hsla| {
                div()
                    .id(id)
                    .w(px(CAPTION_BUTTON))
                    .h_full()
                    .flex()
                    .items_center()
                    .justify_center()
                    .font_family(ICON_FONT)
                    .text_size(px(10.0))
                    .hover(move |s| s.bg(hover))
                    .window_control_area(area)
                    .child(glyph)
            };
        div()
            .flex()
            .h_full()
            .child(caption(
                "min",
                icon::MINIMIZE,
                WindowControlArea::Min,
                colors.hover,
            ))
            .child(caption(
                "max",
                restore,
                WindowControlArea::Max,
                colors.hover,
            ))
            .child(caption(
                "close",
                icon::CLOSE,
                WindowControlArea::Close,
                colors.close_hover,
            ))
    }

    // ---- toolbar -------------------------------------------------------

    fn toolbar(&self, window: &Window, cx: &mut Context<Self>) -> impl IntoElement {
        let loading = self.active_id().is_some_and(|id| self.is_loading(id));
        let (omnibox_x, omnibox_w) = self.omnibox_span;
        div()
            .relative()
            .h(px(TOOLBAR_ROW))
            .w_full()
            .flex()
            .items_center()
            .px(px(8.0))
            .gap(px(2.0))
            .child(self.page_control("back", icon::BACK, cx, |this| this.go_back()))
            .child(self.page_control("forward", icon::FORWARD, cx, |this| this.go_forward()))
            .child(self.page_control(
                "reload",
                if loading { icon::STOP } else { icon::REFRESH },
                cx,
                |this| this.reload_or_stop(),
            ))
            .child(div().flex_1())
            .children(self.vault_pill(window, cx))
            .children(self.permission_pill(cx))
            .child(self.status_pills(cx))
            .child(
                div()
                    .absolute()
                    .left(px(omnibox_x))
                    .top(px((TOOLBAR_ROW - 28.0) / 2.0))
                    .w(px(omnibox_w))
                    .child(self.omnibox(cx)),
            )
    }

    fn omnibox(&self, cx: &mut Context<Self>) -> impl IntoElement {
        let colors = self.colors;
        let editing = self.omnibox_editing;
        let field = div()
            .id("omnibox")
            .w_full()
            .h(px(28.0))
            .flex()
            .items_center()
            .px(px(10.0))
            .rounded(px(self.theme.shape.radius_md))
            .bg(colors.field)
            .text_size(px(13.0))
            .border_1()
            .border_color(colors.focus.opacity(self.springs.borrow_mut().drive(
                "omnibox-ring",
                0.0,
                f32::from(u8::from(editing)),
            )))
            // The field turns Up/Down into its own navigation actions; capture
            // them on the way down so they move through the list instead.
            .capture_action(cx.listener(|this, _: &NavUp, _, cx| {
                if this.move_selection(false, cx) {
                    cx.stop_propagation();
                }
            }))
            .capture_action(cx.listener(|this, _: &NavDown, _, cx| {
                if this.move_selection(true, cx) {
                    cx.stop_propagation();
                }
            }));
        if editing {
            return field
                .child(
                    text_input("omnibox-input")
                        .state(self.omnibox.downgrade())
                        .placeholder("Search, enter address, or type > for commands")
                        .placeholder_color(colors.muted)
                        .caret_color(colors.accent)
                        .caret_blink_interval_500ms()
                        .w_full()
                        .whitespace_nowrap(),
                )
                .into_any_element();
        }
        let (secure, host, path) = self.address_label();
        let placeholder =
            path.is_none() && !secure && self.active_tab().is_none_or(|t| t.url == NEW_TAB_URL);
        field
            .justify_center()
            .gap(px(6.0))
            .cursor_text()
            .on_mouse_down(
                MouseButton::Left,
                cx.listener(|this, _, window, cx| this.click_omnibox(window, cx)),
            )
            .children(self.site_button(secure, cx))
            // Host and path read as one address, so no gap between them.
            .child(
                // The host never truncates; a long path gives way first.
                div()
                    .min_w_0()
                    .overflow_hidden()
                    .flex()
                    .child(
                        div()
                            .flex_none()
                            .whitespace_nowrap()
                            .when(placeholder, |d| d.text_color(colors.muted))
                            .when(!placeholder, |d| d.font_weight(FontWeight::MEDIUM))
                            .child(host),
                    )
                    .children(
                        path.map(|p| div().min_w_0().truncate().text_color(colors.muted).child(p)),
                    ),
            )
            .children(self.reader_button(cx))
            .into_any_element()
    }

    /// A quiet pill beside the omnibox for vault fill, save and unlock. It
    /// sits in the toolbar because nothing can be drawn over the page.
    fn vault_pill(&self, window: &Window, cx: &mut Context<Self>) -> Option<AnyElement> {
        let shown = self.springs.borrow_mut().drive(
            "vault-pill",
            0.0,
            f32::from(u8::from(self.vault.prompt.is_some())),
        );
        let prompt = self.vault.prompt.as_ref()?;
        let colors = self.colors;
        let (label, action): (SharedString, &'static str) = match prompt {
            Prompt::Fill {
                accounts, index, ..
            } => {
                let user = accounts.get(*index).map_or("", |a| a.username.as_str());
                let more = if accounts.len() > 1 {
                    format!("  {}/{}", index + 1, accounts.len())
                } else {
                    String::new()
                };
                (format!("{user}{more}").into(), "Fill")
            }
            Prompt::UnlockToFill { .. } => ("Saved password".into(), "Unlock"),
            Prompt::Save { origin, update, .. } => {
                let verb = if update.is_some() {
                    "Update password for"
                } else {
                    "Save password for"
                };
                (format!("{verb} {}", origin.host()).into(), "Save")
            }
            Prompt::Password { .. } => return Some(self.secret_pill(window, cx)),
        };
        let fill_cycles = matches!(prompt, Prompt::Fill { accounts, .. } if accounts.len() > 1);
        Some(
            pill_frame(colors, self.theme.shape.radius_lg)
                .opacity(shown)
                .ml(px(RISE * (1.0 - shown)))
                .id("vault-pill")
                .child(glyph_text(icon::KEY, 11.0).text_color(colors.muted))
                .child(
                    div()
                        .id("vault-label")
                        .max_w(px(220.0))
                        .truncate()
                        .when(fill_cycles, |d| d.cursor_pointer())
                        .on_click(cx.listener(|this, _, _, cx| {
                            this.next_account();
                            cx.notify();
                        }))
                        .child(label),
                )
                .child(
                    text_button("vault-accept", action, colors).on_click(cx.listener(
                        |this, _, _, cx| {
                            this.accept_vault_prompt(cx);
                            cx.notify();
                        },
                    )),
                )
                .child(
                    self.icon_button("vault-dismiss", icon::CLOSE, 20.0, 8.0, cx)
                        .on_click(cx.listener(|this, _, _, cx| {
                            this.dismiss_vault_prompt();
                            cx.notify();
                        })),
                )
                .into_any_element(),
        )
    }

    fn secret_pill(&self, window: &Window, cx: &mut Context<Self>) -> AnyElement {
        let colors = self.colors;
        let creating = matches!(
            self.vault.prompt,
            Some(Prompt::Password { creating: true, .. })
        );
        let focused = self.vault.secret_focus.is_focused(window);
        let count = self.vault.secret.chars().count();
        let dots: SharedString = if self.vault.busy {
            if creating {
                "Creating vault\u{2026}".into()
            } else {
                "Unlocking\u{2026}".into()
            }
        } else if count == 0 {
            if creating {
                "New master password".into()
            } else {
                "Master password".into()
            }
        } else {
            "\u{2022}".repeat(count.min(24)).into()
        };
        let hint: Option<SharedString> = self.vault.error.map(Into::into);
        pill_frame(colors, self.theme.shape.radius_lg)
            .id("vault-secret")
            .track_focus(&self.vault.secret_focus)
            .on_key_down(cx.listener(|this, e: &gpui::KeyDownEvent, window, cx| {
                this.secret_key(e, window, cx)
            }))
            .on_mouse_down(
                MouseButton::Left,
                cx.listener(|this, _, window, cx| {
                    crate::platform::take_focus(this.hwnd);
                    window.focus(&this.vault.secret_focus, cx);
                }),
            )
            .child(glyph_text(icon::LOCK, 11.0).text_color(colors.muted))
            .child(
                div()
                    .w(px(150.0))
                    .px(px(6.0))
                    .rounded(px(4.0))
                    .bg(colors.field)
                    .when(focused, |d| d.border_1().border_color(colors.focus))
                    .text_color(if count == 0 {
                        colors.muted
                    } else {
                        colors.text
                    })
                    .child(dots),
            )
            .children(hint.map(|h| div().text_color(colors.muted).child(h)))
            .child(
                text_button(
                    "vault-unlock",
                    if creating { "Create" } else { "Unlock" },
                    colors,
                )
                .on_click(cx.listener(|this, _, _, cx| {
                    this.accept_vault_prompt(cx);
                    cx.notify();
                })),
            )
            .child(
                self.icon_button("vault-cancel", icon::CLOSE, 20.0, 8.0, cx)
                    .on_click(cx.listener(|this, _, _, cx| {
                        this.dismiss_vault_prompt();
                        cx.notify();
                    })),
            )
            .into_any_element()
    }

    /// Host prominent, path muted, while not editing; the full URL appears
    /// once the field is focused.
    /// Back, forward and reload: inert and faded on the Settings tab, which
    /// has no page history.
    fn page_control(
        &self,
        id: &'static str,
        glyph: &'static str,
        cx: &mut Context<Self>,
        act: impl Fn(&mut Browser) + 'static,
    ) -> AnyElement {
        let button = self.icon_button(id, glyph, 30.0, 12.0, cx);
        if self.settings_open {
            return button.opacity(0.35).into_any_element();
        }
        button
            .on_click(cx.listener(move |this, _, _, _| act(this)))
            .into_any_element()
    }

    fn address_label(&self) -> (bool, SharedString, Option<SharedString>) {
        if self.settings_open {
            return (false, "Settings".into(), None);
        }
        let Some(tab) = self.active_tab() else {
            return (false, "".into(), None);
        };
        if tab.url == NEW_TAB_URL {
            return (false, "Search or enter address".into(), None);
        }
        let Ok(url) = Url::parse(&tab.url) else {
            return (false, tab.url.clone().into(), None);
        };
        let host = url
            .host_str()
            .unwrap_or(url.as_str())
            .trim_start_matches("www.")
            .to_owned();
        let mut path = url.path().trim_end_matches('/').to_owned();
        if let Some(query) = url.query() {
            path.push('?');
            path.push_str(query);
        }
        let path = (!path.is_empty()).then(|| SharedString::from(path));
        (url.scheme() == "https", host.into(), path)
    }

    fn status_pills(&self, cx: &mut Context<Self>) -> impl IntoElement {
        let colors = self.colors;
        let blocked = self.blocked_count();
        let star = if self.is_bookmarked() {
            icon::STAR_FILLED
        } else {
            icon::STAR
        };
        div()
            .flex()
            .items_center()
            .gap(px(2.0))
            .child(
                self.icon_button("bookmark", star, 30.0, 12.0, cx)
                    .on_click(cx.listener(|this, _, _, cx| {
                        this.toggle_bookmark();
                        cx.notify();
                    })),
            )
            .children(self.toast_pill())
            .children(self.ip_pill(cx))
            .children(self.popup_pill(cx))
            .children(self.download_pill(cx))
            .child(
                self.icon_button("split", icon::SPLIT, 30.0, 12.0, cx)
                    .when(self.split.is_some(), |d| d.text_color(colors.accent))
                    .on_click(cx.listener(|this, _, _, cx| {
                        this.toggle_split();
                        cx.notify();
                    })),
            )
            .child(
                div()
                    .h(px(24.0))
                    .px(px(8.0))
                    .flex()
                    .items_center()
                    .gap(px(5.0))
                    .rounded(px(self.theme.shape.radius_sm))
                    .text_size(px(11.0))
                    .text_color(colors.muted)
                    .child(glyph_text(icon::SHIELD, 11.0))
                    .child(SharedString::from(blocked.to_string())),
            )
            .child(
                self.icon_button("settings", icon::SETTINGS, 30.0, 12.0, cx)
                    .when(self.settings_open, |d| d.text_color(colors.accent))
                    .on_click(cx.listener(|this, _, _, cx| this.open_settings(cx))),
            )
    }

    /// A page tried to open a window without a click. One click opens it
    /// anyway; navigating the page away dismisses the pill.
    fn popup_pill(&self, cx: &mut Context<Self>) -> Option<impl IntoElement> {
        let blocked = self
            .blocked_popup
            .as_ref()
            .is_some_and(|(tab, _)| Some(*tab) == self.active_id());
        let shown =
            self.springs
                .borrow_mut()
                .drive("popup-pill", 0.0, f32::from(u8::from(blocked)));
        if !blocked {
            return None;
        }
        let colors = self.colors;
        Some(
            div()
                .id("popup-blocked")
                .opacity(shown)
                .ml(px(RISE * (1.0 - shown)))
                .h(px(24.0))
                .px(px(8.0))
                .flex()
                .items_center()
                .gap(px(6.0))
                .rounded(px(self.theme.shape.radius_sm))
                .text_size(px(11.0))
                .text_color(colors.muted)
                .hover(move |s| s.bg(colors.hover))
                .active(|s| s.opacity(0.7))
                .on_click(cx.listener(|this, _, _, cx| {
                    this.open_blocked_popup();
                    cx.notify();
                }))
                .child("Pop-up blocked")
                .child(div().text_color(colors.accent).child("Open")),
        )
    }

    /// Progress while downloading, the file name once done. Click opens the list.
    fn download_pill(&self, cx: &mut Context<Self>) -> Option<impl IntoElement> {
        let shown = self.springs.borrow_mut().drive(
            "download-pill",
            0.0,
            f32::from(u8::from(self.downloads.latest().is_some())),
        );
        let latest = self.downloads.latest()?;
        let colors = self.colors;
        let active = self.downloads.active_count() > 0;
        let label: SharedString = if active {
            latest.progress_label().into()
        } else {
            latest.file_name().into()
        };
        Some(
            div()
                .id("downloads")
                .opacity(shown)
                .h(px(24.0))
                .max_w(px(180.0))
                .px(px(8.0))
                .flex()
                .items_center()
                .gap(px(5.0))
                .rounded(px(self.theme.shape.radius_sm))
                .text_size(px(11.0))
                .text_color(if active { colors.text } else { colors.muted })
                .hover(move |s| s.bg(colors.hover))
                .when(self.downloads_open, |d| d.text_color(colors.accent))
                .on_click(cx.listener(|this, _, _, cx| this.open_downloads(cx)))
                .child(glyph_text(icon::DOWNLOAD, 11.0))
                .child(div().truncate().child(label))
                .when(!active, |d| {
                    d.child(
                        div()
                            .id("downloads-clear")
                            .flex_none()
                            .size(px(16.0))
                            .flex()
                            .items_center()
                            .justify_center()
                            .rounded(px(4.0))
                            .font_family(ICON_FONT)
                            .text_size(px(7.0))
                            .hover(move |s| s.bg(colors.hover))
                            // Clearing the list hides the pill; files stay on disk.
                            .on_click(cx.listener(|this, _, _, cx| {
                                cx.stop_propagation();
                                this.downloads.clear_finished();
                                this.close_downloads(cx);
                                cx.notify();
                            }))
                            .child(icon::CLOSE),
                    )
                }),
        )
    }
}

impl Browser {
    /// Suggestions under the omnibox, drawn in the page area while it is
    /// hidden. Same width and x as the omnibox so the list reads as its own.
    fn suggestion_panel(&self, cx: &mut Context<Self>) -> Option<impl IntoElement> {
        let open = self.suggestions_open();
        let reveal = self
            .springs
            .borrow_mut()
            .drive("panel", 0.0, f32::from(u8::from(open)));
        if !open {
            return None;
        }
        let (x, width) = self.omnibox_span;
        let c = self.colors;
        let rows: Vec<AnyElement> = self
            .suggestions
            .iter()
            .enumerate()
            .map(|(i, item)| self.suggestion_row(i, item, cx))
            .collect();
        Some(
            div()
                .flex_1()
                .w_full()
                .relative()
                // The page stays in view, faded into the glass, behind the list.
                .children(
                    self.preview
                        .clone()
                        .map(|image| img(image).absolute().size_full().opacity(0.3)),
                )
                .child(
                    div()
                        .absolute()
                        .left(px(x))
                        .top(px(6.0 - RISE * (1.0 - reveal)))
                        .opacity(reveal)
                        .w(px(width))
                        .p(px(6.0))
                        .rounded(px(self.theme.shape.radius_lg))
                        .bg(c.capsule)
                        .shadow_lg()
                        .text_size(px(13.0))
                        .children(rows),
                ),
        )
    }

    fn suggestion_row(
        &self,
        index: usize,
        item: &Suggestion,
        cx: &mut Context<Self>,
    ) -> AnyElement {
        let c = self.colors;
        let selected = index == self.selected;
        let title: SharedString = item.title.clone().into();
        let detail: SharedString = item.detail.clone().into();
        div()
            .id(("suggestion", index))
            .h(px(34.0))
            .flex()
            .items_center()
            .gap(px(10.0))
            .px(px(10.0))
            .rounded(px((self.theme.shape.radius_lg - 4.0).max(4.0)))
            .when(selected, |d| d.bg(c.selection))
            .when(!selected, |d| d.hover(move |s| s.bg(c.hover)))
            .on_mouse_down(
                MouseButton::Left,
                cx.listener(move |this, _, _, cx| this.choose_suggestion(index, cx)),
            )
            .child(self.suggestion_icon(item))
            .child(div().flex_1().min_w_0().truncate().child(title))
            .child(
                div()
                    .flex_none()
                    .max_w(px(260.0))
                    .truncate()
                    .text_size(px(12.0))
                    .text_color(c.muted)
                    .child(detail),
            )
            .into_any_element()
    }
}

impl Browser {
    /// The site's own icon when we have one, otherwise the row kind's glyph.
    fn suggestion_icon(&self, item: &Suggestion) -> AnyElement {
        let url = match &item.action {
            crate::suggest::Action::Open(url) => Some(url.clone()),
            crate::suggest::Action::SwitchTab(id) => self.saved_tab(*id).map(|t| t.url.clone()),
            crate::suggest::Action::Run(_) => None,
        };
        let icon = url
            .filter(|_| item.kind != Kind::Search)
            .and_then(|u| self.icon_for(&u));
        match icon {
            Some(image) => div()
                .size(px(14.0))
                .flex_none()
                .child(img(image).size_full())
                .into_any_element(),
            None => glyph_text(suggestion_glyph(item.kind), 12.0)
                .text_color(self.colors.muted)
                .into_any_element(),
        }
    }
}

fn suggestion_glyph(kind: Kind) -> &'static str {
    match kind {
        Kind::Go => icon::GLOBE,
        Kind::Search => icon::SEARCH,
        Kind::Tab => icon::TAB,
        Kind::Bookmark => icon::STAR,
        Kind::History => icon::HISTORY,
        Kind::Command => icon::COMMAND,
    }
}

fn pill_frame(colors: crate::style::ChromeColors, radius: f32) -> gpui::Div {
    div()
        .flex_none()
        .h(px(28.0))
        .mr(px(6.0))
        .flex()
        .items_center()
        .gap(px(6.0))
        .pl(px(10.0))
        .pr(px(4.0))
        .rounded(px(radius))
        .bg(colors.capsule)
        .shadow_sm()
        .text_size(px(12.0))
}

/// The pill's one action: accent fill with `on_accent` text, which keeps AA
/// where accent-colored text on the pill would not.
fn text_button(
    id: &'static str,
    label: &'static str,
    colors: crate::style::ChromeColors,
) -> gpui::Stateful<gpui::Div> {
    div()
        .id(id)
        .px(px(10.0))
        .h(px(22.0))
        .flex()
        .items_center()
        .rounded(px(5.0))
        .bg(colors.accent)
        .font_weight(FontWeight::SEMIBOLD)
        .text_color(colors.on_accent)
        .hover(|s| s.opacity(0.88))
        .active(|s| s.opacity(0.7))
        .child(label)
}

/// A restored tab that has not loaded yet has no title; its host reads
/// better in the strip than a full URL.
fn tab_title(tab: &SavedTab) -> String {
    if !tab.title.is_empty() {
        return tab.title.clone();
    }
    if tab.url == NEW_TAB_URL {
        return "New Tab".to_owned();
    }
    Url::parse(&tab.url)
        .ok()
        .and_then(|u| {
            u.host_str()
                .map(|h| h.trim_start_matches("www.").to_owned())
        })
        .unwrap_or_else(|| tab.url.clone())
}

/// The window's drag handle: the empty strip after the tabs. Windows treats
/// a drag area as the caption and swallows clicks inside it, so it must not
/// sit under tabs or buttons.
fn drag_region(left: f32) -> impl IntoElement {
    div()
        .absolute()
        .top_0()
        .bottom_0()
        .right_0()
        .left(px(left))
        .window_control_area(WindowControlArea::Drag)
}

fn glyph_text(glyph: &'static str, size: f32) -> gpui::Div {
    div()
        .font_family(ICON_FONT)
        .text_size(px(size))
        .child(glyph)
}

impl Browser {
    /// 0..1 on a spring while the pointer is over the element keyed `key`.
    pub(crate) fn hover_spring(&self, key: &str) -> f32 {
        let on = self.hovered.as_deref() == Some(key);
        self.springs
            .borrow_mut()
            .drive_keyed(format!("hover-{key}"), 0.0, f32::from(u8::from(on)))
    }

    /// Pairs with `hover_spring`: tracks which keyed element has the pointer.
    pub(crate) fn hover_listener(
        &self,
        key: String,
        cx: &mut Context<Self>,
    ) -> impl Fn(&bool, &mut Window, &mut gpui::App) + 'static {
        cx.listener(move |this, over: &bool, _, cx| {
            if *over {
                this.hovered = Some(key.clone());
            } else if this.hovered.as_deref() == Some(key.as_str()) {
                this.hovered = None;
            }
            cx.notify();
        })
    }

    /// A glyph button whose hover fill fades in and out on the shared spring
    /// instead of snapping, so moving across the toolbar feels continuous.
    fn icon_button(
        &self,
        id: impl Into<gpui::ElementId>,
        glyph: &'static str,
        size: f32,
        glyph_size: f32,
        cx: &mut Context<Self>,
    ) -> gpui::Stateful<gpui::Div> {
        let id: gpui::ElementId = id.into();
        let key = id.to_string();
        let hovered = self.hovered.as_deref() == Some(key.as_str());
        let fill = self.springs.borrow_mut().drive_keyed(
            format!("hover-{key}"),
            0.0,
            f32::from(u8::from(hovered)),
        );
        div()
            .id(id)
            .flex_none()
            .w(px(size))
            .h(px(size.min(28.0)))
            .flex()
            .items_center()
            .justify_center()
            .rounded(px(6.0))
            .font_family(ICON_FONT)
            .text_size(px(glyph_size))
            .bg(self.colors.hover.opacity(fill))
            .active(|s| s.opacity(0.6))
            .on_hover(cx.listener(move |this, over: &bool, _, cx| {
                if *over {
                    this.hovered = Some(key.clone());
                } else if this.hovered.as_deref() == Some(key.as_str()) {
                    this.hovered = None;
                }
                cx.notify();
            }))
            .child(glyph)
    }
}
