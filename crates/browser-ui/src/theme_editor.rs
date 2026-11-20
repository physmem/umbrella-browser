//! Appearance: theme cards with a live miniature of the chrome, and an
//! editor for custom themes. Edits apply to the real chrome as you drag,
//! so the browser itself is the preview; the file is written on release.

use std::cell::RefCell;
use std::collections::HashMap;
use std::rc::Rc;

use browser_core::ThemeId;
use browser_core::spring::MotionStyle;
use browser_core::theme::{Rgba, ThemeRegistry, ThemeTokens};
use gpui::{
    AnyElement, Bounds, ColorExt, Context, FontWeight, InteractiveElement, IntoElement,
    MouseButton, ParentElement, Pixels, SharedString, StatefulInteractiveElement, Styled, Window,
    canvas, div, linear_color_stop, linear_gradient, prelude::*, px, relative,
};
use gpui_ce_elements::editable_text::text_input;

use crate::browser::Browser;
use crate::settings_ui::{block, row, segmented, toggle_for};
use crate::style::{ICON_FONT, hsla, icon, mix};

const CARD_WIDTH: f32 = 156.0;
const MONO_FONT: &str = "Consolas";
/// The glass slider runs from half see-through to solid.
const GLASS_MIN: f32 = 0.5;
const SQUARE_W: f32 = 220.0;
const SQUARE_H: f32 = 140.0;

/// A colour a custom theme lets you change. Some roles move more than one
/// token so the chrome stays coherent (the glass tint follows the window).
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub(crate) enum ColorRole {
    Background,
    Surface,
    Text,
    SecondaryText,
    Accent,
    Focus,
}

const ROLES: &[(ColorRole, &str, &str)] = &[
    (
        ColorRole::Background,
        "Background",
        "The window and the glass behind the tabs.",
    ),
    (
        ColorRole::Surface,
        "Surface",
        "The active tab, pills and panels.",
    ),
    (ColorRole::Text, "Text", "Titles, the address and labels."),
    (
        ColorRole::SecondaryText,
        "Secondary text",
        "Paths, inactive tabs and hints.",
    ),
    (
        ColorRole::Accent,
        "Accent",
        "Selection, switches and the one main button.",
    ),
    (
        ColorRole::Focus,
        "Focus ring",
        "The outline on a field you are typing in.",
    ),
];

impl ColorRole {
    fn get(self, t: &ThemeTokens) -> Rgba {
        let c = &t.colors;
        match self {
            Self::Background => c.window,
            Self::Surface => c.surface_raised,
            Self::Text => c.text,
            Self::SecondaryText => c.text_muted,
            Self::Accent => c.accent,
            Self::Focus => c.focus,
        }
    }

    fn set(self, t: &mut ThemeTokens, color: Rgba) {
        let c = &mut t.colors;
        match self {
            Self::Background => {
                c.window = color;
                t.glass.tint = color;
            }
            Self::Surface => {
                c.surface_raised = color;
                c.surface = color;
            }
            Self::Text => c.text = color,
            Self::SecondaryText => c.text_muted = color,
            Self::Accent => {
                c.selection = color.with_alpha(c.selection.alpha());
                c.accent = color;
            }
            Self::Focus => c.focus = color,
        }
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub(crate) enum Slider {
    /// The saturation/brightness square.
    Field,
    Hue,
    Glass,
}

/// Where each slider's track was laid out last frame, for pointer maths.
pub(crate) type SliderBounds = Rc<RefCell<HashMap<Slider, Bounds<Pixels>>>>;

impl Browser {
    pub(crate) fn appearance_pane(&self, cx: &mut Context<Self>) -> AnyElement {
        let c = self.colors;
        let current = self.current_theme_id();
        let custom = !ThemeRegistry::is_preset(&current);
        let calm = self.settings.motion == MotionStyle::Calm;
        let theme_actions = div()
            .flex()
            .items_center()
            .gap(px(8.0))
            .pt(px(14.0))
            .when(!custom, |d| {
                d.child(
                    self.action_button("customize-theme", "Customize colours", true, cx)
                        .on_click(
                            cx.listener(|this, _, window, cx| this.copy_theme(false, window, cx)),
                        ),
                )
                .child(
                    div().text_size(px(12.0)).text_color(c.muted).child(
                        "Built-in themes stay as they are; this makes your own copy to change.",
                    ),
                )
            })
            .when(custom, |d| {
                d.child(
                    self.action_button("duplicate-theme", "Duplicate", false, cx)
                        .on_click(
                            cx.listener(|this, _, window, cx| this.copy_theme(false, window, cx)),
                        ),
                )
                .child(
                    self.action_button("delete-theme", "Delete theme", false, cx)
                        .on_click(cx.listener(|this, _, window, cx| this.delete_theme(window, cx))),
                )
            });
        let mut out = div().flex().flex_col().gap(px(28.0)).child(self.group(
            "Theme",
            vec![block(
                    div()
                        .flex()
                        .flex_col()
                        .child(self.theme_cards(&current, cx))
                        .child(theme_actions),
                )],
        ));
        if custom {
            out = out.child(self.group("Colours", self.color_rows(&current, cx)));
        }
        out.child(self.group(
            "Motion and glass",
            vec![
                row(
                    "Motion",
                    "Calm settles without overshoot. Lively adds one small, quick settle.",
                    c,
                    segmented(
                        &self.springs,
                        "motion",
                        &["Calm", "Lively"],
                        usize::from(!calm),
                        c,
                        cx.listener(|this, index: &usize, _, cx| {
                            let style = if *index == 0 {
                                MotionStyle::Calm
                            } else {
                                MotionStyle::Lively
                            };
                            this.update_settings(cx, |s| s.motion = style);
                        }),
                    ),
                ),
                row(
                    "Reduced glass",
                    "Solid surfaces instead of frosted glass. Saves a little power.",
                    c,
                    toggle_for("reduced-glass", self.settings.reduced_glass, c, &self.springs)
                        .on_click(cx.listener(|this, _, _, cx| {
                            this.update_settings(cx, |s| s.reduced_glass = !s.reduced_glass)
                        })),
                ),
            ],
        ))
        .into_any_element()
    }

    fn theme_cards(&self, current: &ThemeId, cx: &mut Context<Self>) -> impl IntoElement {
        let ids: Vec<ThemeId> = self.themes.ids().cloned().collect();
        div()
            .flex()
            .flex_wrap()
            .gap(px(12.0))
            .children(ids.into_iter().filter_map(|id| {
                let tokens = self.themes.get(&id)?.clone();
                Some(self.theme_card(id.clone(), &tokens, &id == current, cx))
            }))
            .child(self.new_theme_card(cx))
    }

    /// A small drawing of the chrome in that theme's own colours.
    fn theme_card(
        &self,
        id: ThemeId,
        t: &ThemeTokens,
        selected: bool,
        cx: &mut Context<Self>,
    ) -> AnyElement {
        let c = self.colors;
        let p = &t.colors;
        let name: SharedString = t.name.clone().into();
        let kind = if ThemeRegistry::is_preset(&id) {
            "Built-in"
        } else {
            "Custom"
        };
        let lift = self.springs.borrow_mut().drive_keyed(
            format!("theme-card-{}", id.as_str()),
            0.0,
            f32::from(u8::from(selected)),
        );
        let glass = hsla(t.glass.tint.with_alpha(1.0));
        let preview = div()
            .h(px(78.0))
            .w_full()
            .flex()
            .flex_col()
            .bg(hsla(p.window))
            .child(
                div()
                    .h(px(46.0))
                    .w_full()
                    .px(px(8.0))
                    .pt(px(8.0))
                    .flex()
                    .flex_col()
                    .gap(px(6.0))
                    .bg(glass)
                    .border_b_1()
                    .border_color(hsla(p.border))
                    .child(
                        div()
                            .flex()
                            .gap(px(4.0))
                            .child(
                                div()
                                    .w(px(46.0))
                                    .h(px(12.0))
                                    .rounded(px(4.0))
                                    .bg(hsla(p.surface_raised))
                                    .border_1()
                                    .border_color(hsla(p.accent.with_alpha(0.5)))
                                    .flex()
                                    .items_center()
                                    .px(px(4.0))
                                    .child(
                                        div()
                                            .w(px(24.0))
                                            .h(px(3.0))
                                            .rounded_full()
                                            .bg(hsla(p.text)),
                                    ),
                            )
                            .child(
                                div()
                                    .w(px(40.0))
                                    .h(px(12.0))
                                    .flex()
                                    .items_center()
                                    .px(px(4.0))
                                    .child(
                                        div()
                                            .w(px(22.0))
                                            .h(px(3.0))
                                            .rounded_full()
                                            .bg(hsla(p.text_muted)),
                                    ),
                            ),
                    )
                    .child(
                        div()
                            .mx(px(18.0))
                            .h(px(10.0))
                            .rounded(px(4.0))
                            .bg(hsla(p.text.with_alpha(0.08)))
                            .flex()
                            .items_center()
                            .justify_center()
                            .child(
                                div()
                                    .w(px(36.0))
                                    .h(px(3.0))
                                    .rounded_full()
                                    .bg(hsla(p.text_muted)),
                            ),
                    ),
            )
            .child(
                div()
                    .flex_1()
                    .px(px(10.0))
                    .flex()
                    .items_center()
                    .gap(px(6.0))
                    .child(div().size(px(8.0)).rounded_full().bg(hsla(p.accent)))
                    .child(
                        div()
                            .w(px(50.0))
                            .h(px(3.0))
                            .rounded_full()
                            .bg(hsla(p.text_muted)),
                    ),
            );
        let target = id.clone();
        div()
            .id(SharedString::from(format!("theme-{}", id.as_str())))
            .w(px(CARD_WIDTH))
            .flex()
            .flex_col()
            .gap(px(8.0))
            .cursor_pointer()
            .on_click(
                cx.listener(move |this, _, window, cx| {
                    this.select_theme(target.clone(), window, cx)
                }),
            )
            .child(
                div()
                    .w_full()
                    .rounded(px(10.0))
                    .overflow_hidden()
                    .border_2()
                    .border_color(mix(c.border, c.accent, lift))
                    .shadow_sm()
                    .mt(px(-2.0 * lift))
                    .hover(move |s| s.border_color(if selected { c.accent } else { c.muted }))
                    .child(preview),
            )
            .child(
                div()
                    .px(px(2.0))
                    .flex()
                    .flex_col()
                    .gap(px(1.0))
                    .child(
                        div()
                            .truncate()
                            .when(selected, |d| d.font_weight(FontWeight::MEDIUM))
                            .child(name),
                    )
                    .child(div().text_size(px(12.0)).text_color(c.muted).child(kind)),
            )
            .into_any_element()
    }

    fn new_theme_card(&self, cx: &mut Context<Self>) -> AnyElement {
        let c = self.colors;
        div()
            .id("theme-new")
            .w(px(CARD_WIDTH))
            .flex()
            .flex_col()
            .gap(px(8.0))
            .cursor_pointer()
            .on_click(cx.listener(|this, _, window, cx| this.copy_theme(true, window, cx)))
            .child(
                div()
                    .h(px(82.0))
                    .w_full()
                    .rounded(px(10.0))
                    .border_2()
                    .border_color(c.border)
                    .bg(c.field)
                    .flex()
                    .items_center()
                    .justify_center()
                    .font_family(ICON_FONT)
                    .text_size(px(16.0))
                    .text_color(c.muted)
                    .hover(move |s| s.border_color(c.muted).text_color(c.text))
                    .child(icon::ADD),
            )
            .child(
                div()
                    .px(px(2.0))
                    .flex()
                    .flex_col()
                    .gap(px(1.0))
                    .child("New theme")
                    .child(
                        div()
                            .text_size(px(12.0))
                            .text_color(c.muted)
                            .child("Starts from the current one"),
                    ),
            )
            .into_any_element()
    }

    fn color_rows(&self, current: &ThemeId, cx: &mut Context<Self>) -> Vec<AnyElement> {
        let c = self.colors;
        let Some(tokens) = self.themes.get(current).cloned() else {
            return Vec::new();
        };
        let role = self.edit_role();
        let mut rows = vec![
            row(
                "Name",
                "Shown on the theme card.",
                c,
                field_box(c, 220.0).child(
                    text_input("theme-name")
                        .state(self.theme_name_field.downgrade())
                        .caret_color(c.accent)
                        .caret_blink_interval_500ms()
                        .w_full()
                        .whitespace_nowrap(),
                ),
            ),
            block(
                div()
                    .flex()
                    .flex_col()
                    .gap(px(18.0))
                    .child(div().flex().flex_wrap().gap(px(8.0)).children(
                        ROLES.iter().map(|(r, label, _)| {
                            self.role_chip(*r, label, &tokens, *r == role, cx)
                        }),
                    ))
                    .child(
                        div()
                            .text_size(px(12.0))
                            .text_color(c.muted)
                            .child(role_detail(role)),
                    )
                    .child(self.color_picker(role, &tokens, cx)),
            ),
        ];
        let glass_t = ((tokens.glass.opacity - GLASS_MIN) / (1.0 - GLASS_MIN)).clamp(0.0, 1.0);
        rows.push(row(
            "Glass",
            "How much of the desktop shows through the tab strip and toolbar.",
            c,
            div().w(px(200.0)).child(self.slider(
                Slider::Glass,
                glass_t,
                vec![(tokens.glass.tint.with_alpha(GLASS_MIN), tokens.glass.tint)],
                hsla(tokens.glass.tint),
                cx,
            )),
        ));
        let issues = tokens.contrast_issues();
        if !issues.is_empty() {
            rows.push(block(
                div()
                    .flex()
                    .flex_col()
                    .gap(px(4.0))
                    .text_size(px(12.0))
                    .child(
                        div()
                            .font_weight(FontWeight::MEDIUM)
                            .child("Some text will be hard to read"),
                    )
                    .children(issues.into_iter().map(|issue| {
                        let text: SharedString = format!(
                            "{} is {:.1}:1, aim for at least 4.5:1.",
                            capitalise(issue.pair),
                            issue.ratio
                        )
                        .into();
                        div().text_color(c.muted).child(text)
                    })),
            ));
        }
        rows
    }

    /// A pill naming one colour role, with its swatch. The picker below
    /// edits whichever chip is selected.
    fn role_chip(
        &self,
        role: ColorRole,
        label: &'static str,
        t: &ThemeTokens,
        selected: bool,
        cx: &mut Context<Self>,
    ) -> AnyElement {
        let c = self.colors;
        let key = format!("chip-{label}");
        let hover = self.hover_spring(&key);
        let picked = self.springs.borrow_mut().drive_keyed(
            format!("chip-on-{label}"),
            0.0,
            f32::from(u8::from(selected)),
        );
        let swatch = hsla(role.get(t).with_alpha(1.0));
        div()
            .id(SharedString::from(key.clone()))
            .h(px(34.0))
            .pl(px(5.0))
            .pr(px(12.0))
            .flex()
            .items_center()
            .gap(px(8.0))
            .rounded_full()
            .cursor_pointer()
            .border_1()
            .border_color(mix(c.border, c.accent, picked))
            .bg(mix(c.hover.opacity(hover), c.selection, picked))
            .text_size(px(12.0))
            .when(selected, |d| d.font_weight(FontWeight::MEDIUM))
            .when(!selected, |d| d.text_color(c.muted))
            .on_hover(self.hover_listener(key, cx))
            .active(|s| s.opacity(0.8))
            .on_click(cx.listener(move |this, _, _, cx| this.select_role(role, cx)))
            .child(
                div()
                    .size(px(24.0 + 2.0 * hover))
                    .rounded_full()
                    .bg(swatch)
                    .border_1()
                    .border_color(c.border),
            )
            .child(label)
            .into_any_element()
    }

    /// A saturation/brightness square, a hue bar, a hex field and a few
    /// suggestions, all for the selected role.
    fn color_picker(&self, role: ColorRole, t: &ThemeTokens, cx: &mut Context<Self>) -> AnyElement {
        let c = self.colors;
        let color = role.get(t).with_alpha(1.0);
        let (_, s, v) = color.to_hsv();
        let hue = self.edit_hue;
        let pure = Rgba::hex(0xff0000).with_hsv(hue, 1.0, 1.0);
        let hue_stops: Vec<(Rgba, Rgba)> = (0..6)
            .map(|i| {
                let from = pure.with_hsv(i as f32 * 60.0, 1.0, 1.0);
                let to = pure.with_hsv((i + 1) as f32 * 60.0, 1.0, 1.0);
                (from, to)
            })
            .collect();
        let bounds = self.slider_bounds.clone();
        let square = div()
            .id("color-field")
            .relative()
            .w(px(SQUARE_W))
            .h(px(SQUARE_H))
            .flex_none()
            .cursor_crosshair()
            .on_mouse_down(
                MouseButton::Left,
                cx.listener(|this, e: &gpui::MouseDownEvent, window, cx| {
                    this.slider_drag = Some(Slider::Field);
                    this.slide(Slider::Field, e.position, window, cx);
                }),
            )
            // GPUI clips children to a rectangle, not to rounded corners, so
            // every layer carries the radius. The knob is a sibling so it can
            // sit on the edge without being cut off.
            .child(
                div()
                    .absolute()
                    .size_full()
                    .rounded(px(10.0))
                    .overflow_hidden()
                    .bg(hsla(pure))
                    .child(
                        div()
                            .absolute()
                            .size_full()
                            .rounded(px(10.0))
                            .bg(linear_gradient(
                                90.0,
                                linear_color_stop(gpui::white(), 0.0),
                                linear_color_stop(gpui::white().opacity(0.0), 1.0),
                            )),
                    )
                    .child(
                        div()
                            .absolute()
                            .size_full()
                            .rounded(px(10.0))
                            .bg(linear_gradient(
                                180.0,
                                linear_color_stop(gpui::black().opacity(0.0), 0.0),
                                linear_color_stop(gpui::black(), 1.0),
                            )),
                    ),
            )
            .child(
                canvas(
                    move |b, _, _| {
                        bounds.borrow_mut().insert(Slider::Field, b);
                    },
                    |_, _, _, _| {},
                )
                .absolute()
                .size_full(),
            )
            .child(
                div()
                    .absolute()
                    .left(relative(s))
                    .top(relative(1.0 - v))
                    .ml(px(-8.0))
                    .mt(px(-8.0))
                    .size(px(16.0))
                    .rounded_full()
                    .border_2()
                    .border_color(gpui::white())
                    .bg(hsla(color))
                    .shadow_md(),
            );
        let suggestions = suggestions_for(role);
        let current_hex = color.to_hex();
        let right = div()
            .flex_1()
            .min_w_0()
            .flex()
            .flex_col()
            .gap(px(14.0))
            .child(self.slider(Slider::Hue, hue / 360.0, hue_stops, hsla(pure), cx))
            .child(
                div()
                    .flex()
                    .items_center()
                    .gap(px(10.0))
                    .child(
                        div()
                            .size(px(30.0))
                            .flex_none()
                            .rounded(px(8.0))
                            .bg(hsla(color))
                            .border_1()
                            .border_color(c.border),
                    )
                    .child(
                        field_box(c, 116.0).font_family(MONO_FONT).child(
                            text_input("theme-hex")
                                .state(self.theme_hex_field.downgrade())
                                .placeholder("#rrggbb")
                                .placeholder_color(c.muted)
                                .caret_color(c.accent)
                                .caret_blink_interval_500ms()
                                .w_full()
                                .whitespace_nowrap(),
                        ),
                    ),
            )
            .child(div().flex().flex_wrap().gap(px(8.0)).children(
                suggestions.iter().enumerate().map(|(i, hex)| {
                    let swatch = Rgba::hex(*hex);
                    let on = swatch.to_hex() == current_hex;
                    let key = format!("suggest-{i}");
                    let hover = self.hover_spring(&key);
                    div()
                        .id(SharedString::from(key.clone()))
                        .size(px(22.0))
                        .rounded_full()
                        .cursor_pointer()
                        .bg(hsla(swatch))
                        .border_2()
                        .border_color(if on {
                            c.text
                        } else {
                            c.border.opacity(0.5 + 0.5 * hover)
                        })
                        .mt(px(-2.0 * hover))
                        .on_hover(self.hover_listener(key, cx))
                        .on_click(cx.listener(move |this, _, window, cx| {
                            this.apply_color(swatch, window, cx)
                        }))
                }),
            ));
        div()
            .flex()
            .items_start()
            .gap(px(18.0))
            .child(square)
            .child(right)
            .into_any_element()
    }

    /// A thin rounded track painted from `stops` (one gradient per segment)
    /// with a knob at `t`. Pressing anywhere jumps there and starts a drag.
    fn slider(
        &self,
        kind: Slider,
        t: f32,
        stops: Vec<(Rgba, Rgba)>,
        knob: gpui::Hsla,
        cx: &mut Context<Self>,
    ) -> AnyElement {
        let bounds = self.slider_bounds.clone();
        div()
            .id(SharedString::from(format!("slider-{kind:?}")))
            .relative()
            .h(px(20.0))
            .w_full()
            .flex()
            .items_center()
            .cursor_pointer()
            .on_mouse_down(
                MouseButton::Left,
                cx.listener(move |this, e: &gpui::MouseDownEvent, window, cx| {
                    this.slider_drag = Some(kind);
                    this.slide(kind, e.position, window, cx);
                }),
            )
            .child(
                div()
                    .relative()
                    .w_full()
                    .h(px(8.0))
                    .rounded_full()
                    .overflow_hidden()
                    .bg(self.colors.field)
                    .border_1()
                    .border_color(self.colors.border)
                    .flex()
                    .children(stops.into_iter().map(|(from, to)| {
                        div().flex_1().h_full().bg(linear_gradient(
                            90.0,
                            linear_color_stop(hsla(from), 0.0),
                            linear_color_stop(hsla(to), 1.0),
                        ))
                    }))
                    .child(
                        canvas(
                            move |b, _, _| {
                                bounds.borrow_mut().insert(kind, b);
                            },
                            |_, _, _, _| {},
                        )
                        .absolute()
                        .size_full(),
                    ),
            )
            .child(
                div()
                    .absolute()
                    .top(px(2.0))
                    .left(relative(t.clamp(0.0, 1.0)))
                    .ml(px(-8.0))
                    .size(px(16.0))
                    .rounded_full()
                    .border_2()
                    .border_color(gpui::white())
                    .bg(knob)
                    .shadow_md(),
            )
            .into_any_element()
    }

    // ---- actions -------------------------------------------------------

    fn select_theme(&mut self, id: ThemeId, window: &mut Window, cx: &mut Context<Self>) {
        self.theme_edit_role = None;
        if let Some(name) = self.themes.get(&id).map(|t| t.name.clone()) {
            self.theme_name_field
                .update(cx, |field, cx| field.emplace(&name, cx));
        }
        self.update_settings(cx, move |s| s.theme = Some(id));
        self.apply_theme(window);
        self.load_hue();
        self.sync_hex_field(cx);
    }

    /// Copies the current theme into a new custom one and selects it.
    /// `fresh` names it "My theme"; otherwise "<name> custom".
    fn copy_theme(&mut self, fresh: bool, window: &mut Window, cx: &mut Context<Self>) {
        let current = self.current_theme_id();
        let Some(mut tokens) = self.themes.get(&current).cloned() else {
            return;
        };
        tokens.name = if fresh {
            "My theme".to_owned()
        } else {
            format!("{} custom", tokens.name)
        };
        let id = self.themes.free_id(&tokens.name);
        if let Err(e) = self.themes.save_custom(&self.paths.themes(), &id, tokens) {
            eprintln!("theme save failed: {e}");
            return;
        }
        self.select_theme(id, window, cx);
    }

    fn delete_theme(&mut self, window: &mut Window, cx: &mut Context<Self>) {
        let current = self.current_theme_id();
        if let Err(e) = self.themes.delete_custom(&self.paths.themes(), &current) {
            eprintln!("theme delete failed: {e}");
            return;
        }
        self.theme_edit_role = None;
        self.update_settings(cx, |s| s.theme = None);
        self.apply_theme(window);
    }

    /// Fills the name and hex fields from the current theme; run when the
    /// editor comes into view so it never shows blank fields.
    pub(crate) fn prime_theme_fields(&mut self, cx: &mut Context<Self>) {
        let current = self.current_theme_id();
        if let Some(name) = self.themes.get(&current).map(|t| t.name.clone()) {
            self.theme_name_field
                .update(cx, |field, cx| field.emplace(&name, cx));
        }
        self.load_hue();
        self.sync_hex_field(cx);
    }

    pub(crate) fn edit_role(&self) -> ColorRole {
        self.theme_edit_role.unwrap_or(ColorRole::Accent)
    }

    fn select_role(&mut self, role: ColorRole, cx: &mut Context<Self>) {
        self.theme_edit_role = Some(role);
        self.load_hue();
        self.sync_hex_field(cx);
        cx.notify();
    }

    /// Takes the hue from the role's colour, unless it is grey and has none.
    fn load_hue(&mut self) {
        let role = self.edit_role();
        let current = self.current_theme_id();
        if let Some((h, s, _)) = self.themes.get(&current).map(|t| role.get(t).to_hsv())
            && s > 0.02
        {
            self.edit_hue = h;
        }
    }

    /// Moves `kind` to the pointer and applies it live.
    pub(crate) fn slide(
        &mut self,
        kind: Slider,
        pos: gpui::Point<Pixels>,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) {
        let Some(b) = self.slider_bounds.borrow().get(&kind).copied() else {
            return;
        };
        let w = f32::from(b.size.width).max(1.0);
        let h = f32::from(b.size.height).max(1.0);
        let tx = ((f32::from(pos.x) - f32::from(b.origin.x)) / w).clamp(0.0, 1.0);
        let ty = ((f32::from(pos.y) - f32::from(b.origin.y)) / h).clamp(0.0, 1.0);
        if kind == Slider::Hue {
            self.edit_hue = tx * 360.0;
        }
        let hue = self.edit_hue;
        self.edit_theme(window, cx, false, move |tokens, role| match kind {
            Slider::Glass => tokens.glass.opacity = GLASS_MIN + (1.0 - GLASS_MIN) * tx,
            Slider::Hue => {
                let color = role.get(tokens);
                let (_, s, v) = color.to_hsv();
                role.set(tokens, color.with_hsv(hue, s, v));
            }
            Slider::Field => {
                let color = role.get(tokens);
                role.set(tokens, color.with_hsv(hue, tx, 1.0 - ty));
            }
        });
        self.sync_hex_field(cx);
    }

    /// Releasing a slider writes the theme file once.
    pub(crate) fn end_slide(&mut self, window: &mut Window, cx: &mut Context<Self>) {
        if self.slider_drag.take().is_some() {
            self.edit_theme(window, cx, true, |_, _| {});
        }
    }

    fn apply_color(&mut self, color: Rgba, window: &mut Window, cx: &mut Context<Self>) {
        let (h, s, _) = color.to_hsv();
        if s > 0.02 {
            self.edit_hue = h;
        }
        self.edit_theme(window, cx, true, move |tokens, role| {
            role.set(tokens, color)
        });
        self.sync_hex_field(cx);
    }

    pub(crate) fn rename_theme(&mut self, cx: &mut Context<Self>) {
        let name = self.theme_name_field.read(cx).as_str().trim().to_owned();
        let current = self.current_theme_id();
        let unchanged = self.themes.get(&current).is_none_or(|t| t.name == name);
        if name.is_empty() || unchanged || ThemeRegistry::is_preset(&current) {
            return;
        }
        let Some(mut tokens) = self.themes.get(&current).cloned() else {
            return;
        };
        tokens.name = name;
        if let Err(e) = self
            .themes
            .save_custom(&self.paths.themes(), &current, tokens)
        {
            eprintln!("theme save failed: {e}");
        }
        cx.notify();
    }

    /// Applies a typed `#rrggbb` to the open colour role.
    pub(crate) fn hex_typed(&mut self, cx: &mut Context<Self>) {
        let text = self.theme_hex_field.read(cx).as_str().trim().to_owned();
        let digits = text.trim_start_matches('#');
        if digits.len() != 6 {
            return;
        }
        let Ok(color) = Rgba::parse(&format!("#{digits}")) else {
            return;
        };
        let role = self.edit_role();
        let current = self.current_theme_id();
        if self
            .themes
            .get(&current)
            .map(|t| role.get(t).with_alpha(1.0))
            == Some(color)
        {
            return;
        }
        let (h, s, _) = color.to_hsv();
        if s > 0.02 {
            self.edit_hue = h;
        }
        self.defer_in_window(cx, move |b, window, cx| {
            b.edit_theme(window, cx, true, move |tokens, role| {
                role.set(tokens, color)
            });
        });
    }

    fn edit_theme(
        &mut self,
        window: &mut Window,
        cx: &mut Context<Self>,
        persist: bool,
        change: impl FnOnce(&mut ThemeTokens, ColorRole),
    ) {
        let current = self.current_theme_id();
        if ThemeRegistry::is_preset(&current) {
            return;
        }
        let Some(mut tokens) = self.themes.get(&current).cloned() else {
            return;
        };
        change(&mut tokens, self.edit_role());
        let result = if persist {
            self.themes
                .save_custom(&self.paths.themes(), &current, tokens)
                .map(|_| ())
        } else {
            self.themes.put(&current, tokens)
        };
        if let Err(e) = result {
            eprintln!("theme edit failed: {e}");
            return;
        }
        self.apply_theme(window);
        cx.notify();
    }

    fn sync_hex_field(&mut self, cx: &mut Context<Self>) {
        let role = self.edit_role();
        let current = self.current_theme_id();
        if let Some(color) = self.themes.get(&current).map(|t| role.get(t)) {
            let hex = color.with_alpha(1.0).to_hex();
            self.theme_hex_field
                .update(cx, |field, cx| field.emplace(&hex, cx));
        }
    }
}

fn role_detail(role: ColorRole) -> &'static str {
    ROLES
        .iter()
        .find(|(r, _, _)| *r == role)
        .map_or("", |(_, _, detail)| *detail)
}

/// A few good starting points per role, so most edits need one click.
fn suggestions_for(role: ColorRole) -> &'static [u32] {
    match role {
        ColorRole::Background | ColorRole::Surface => &[
            0x0b0b0f, 0x1c1c1e, 0x2c2c2e, 0x11111b, 0x1e1e2e, 0x1a1b26, 0x2e3440, 0x24273a,
        ],
        ColorRole::Text | ColorRole::SecondaryText => &[
            0xffffff, 0xf5f5f7, 0xe5e5ea, 0xaeaeb2, 0xcdd6f4, 0xa6adc8, 0xc0caf5, 0xd8dee9,
        ],
        ColorRole::Accent | ColorRole::Focus => &[
            0x0a84ff, 0x5e5ce6, 0xbf5af2, 0xff375f, 0xff9f0a, 0xffd60a, 0x30d158, 0x64d2ff,
        ],
    }
}

fn field_box(c: crate::style::ChromeColors, width: f32) -> gpui::Div {
    div()
        .w(px(width))
        .h(px(30.0))
        .px(px(10.0))
        .flex()
        .items_center()
        .rounded(px(7.0))
        .bg(c.field)
        .border_1()
        .border_color(c.border)
}

fn capitalise(text: &str) -> String {
    let mut chars = text.chars();
    match chars.next() {
        Some(first) => first.to_uppercase().chain(chars).collect(),
        None => String::new(),
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use browser_core::theme::presets;

    #[test]
    fn background_moves_the_glass_and_accent_keeps_selection_alpha() {
        let mut t = presets::nord();
        let alpha = t.colors.selection.alpha();
        ColorRole::Background.set(&mut t, Rgba::hex(0x101820));
        assert_eq!(t.glass.tint, Rgba::hex(0x101820));
        ColorRole::Accent.set(&mut t, Rgba::hex(0xbf616a));
        assert!((t.colors.selection.alpha() - alpha).abs() < 0.01);
        assert_eq!(ColorRole::Accent.get(&t), Rgba::hex(0xbf616a));
    }

    #[test]
    fn capitalises_contrast_pairs() {
        assert_eq!(capitalise("muted text on chrome"), "Muted text on chrome");
    }
}
