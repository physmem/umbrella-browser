//! The new-tab start page, drawn natively in the page area: pinned
//! favorites, frequently visited sites, and a background of the user's
//! choosing. A new tab has no WebView2 until the user goes somewhere.

use std::path::PathBuf;

use browser_core::settings::StartBackground;
use gpui::{
    AnyElement, Context, FontWeight, InteractiveElement, IntoElement, MouseButton, ObjectFit,
    ParentElement, PathPromptOptions, SharedString, StatefulInteractiveElement, Styled,
    StyledImage, div, img, linear_color_stop, linear_gradient, prelude::*, px,
};

use crate::browser::{Browser, NEW_TAB_URL};
use crate::style::{ICON_FONT, icon};

const TILE: f32 = 84.0;
const TILE_ICON: f32 = 64.0;
const FREQUENT: usize = 8;
const COLUMN: f32 = 760.0;

/// Built-in backgrounds: (from, to, angle). Colors are 0xRRGGBB.
pub const GRADIENTS: &[(u32, u32, f32)] = &[
    (0x1f2a44, 0x6b3f69, 135.0),
    (0x0f3443, 0x34e89e, 160.0),
    (0x42275a, 0x734b6d, 120.0),
    (0x2c3e50, 0xfd746c, 145.0),
    (0x141e30, 0x243b55, 180.0),
];

impl Browser {
    /// The active tab is a fresh tab with nothing else covering the page area.
    pub(crate) fn on_start_page(&self) -> bool {
        self.active_tab().is_some_and(|t| t.url == NEW_TAB_URL)
            && !self.suggestions_open()
            && !self.settings_open
            && !self.downloads_open
            && !self.page_info_open
    }

    pub(crate) fn start_page(&self, cx: &mut Context<Self>) -> Option<AnyElement> {
        let shown = self.on_start_page();
        let reveal = self
            .springs
            .borrow_mut()
            .drive("start", 0.0, f32::from(u8::from(shown)));
        if !shown {
            return None;
        }
        let page = &self.settings.start_page;
        let column = div()
            .w(px(COLUMN))
            .max_w_full()
            .pt(px(72.0 - crate::chrome::RISE * 2.0 * (1.0 - reveal)))
            .flex()
            .flex_col()
            .gap(px(36.0))
            .when(page.show_favorites, |d| d.child(self.favorites_section(cx)))
            .when(page.show_frequent, |d| {
                d.children(self.frequent_section(cx))
            });
        Some(
            div()
                .id("start-page")
                .on_mouse_down(
                    gpui::MouseButton::Left,
                    cx.listener(|this, e: &gpui::MouseDownEvent, window, cx| {
                        this.press_outside_omnibox(e.position, window, cx)
                    }),
                )
                .flex_1()
                .w_full()
                .relative()
                .overflow_hidden()
                .child(self.start_background())
                .child(
                    div()
                        .id("start-scroll")
                        .absolute()
                        .size_full()
                        .overflow_y_scroll()
                        .opacity(reveal)
                        .flex()
                        .justify_center()
                        .child(column),
                )
                .child(self.customize_corner(cx))
                .into_any_element(),
        )
    }

    fn start_background(&self) -> AnyElement {
        let layer = div().absolute().size_full();
        match &self.settings.start_page.background {
            StartBackground::Glass => layer.into_any_element(),
            StartBackground::Gradient(i) => {
                let (from, to, angle) = GRADIENTS[*i % GRADIENTS.len()];
                layer
                    .bg(linear_gradient(
                        angle,
                        linear_color_stop(gpui::rgb(from), 0.0),
                        linear_color_stop(gpui::rgb(to), 1.0),
                    ))
                    .into_any_element()
            }
            StartBackground::Image(path) => {
                // A soft wash of the theme's window color keeps tiles legible
                // on any photo.
                let wash = crate::style::hsla(self.theme.colors.window.with_alpha(0.35));
                layer
                    .child(
                        img(PathBuf::from(path))
                            .absolute()
                            .size_full()
                            .object_fit(ObjectFit::Cover),
                    )
                    .child(div().absolute().size_full().bg(wash))
                    .into_any_element()
            }
        }
    }

    fn favorites_section(&self, cx: &mut Context<Self>) -> impl IntoElement {
        let tiles: Vec<AnyElement> = self
            .settings
            .start_page
            .favorites
            .iter()
            .enumerate()
            .map(|(i, f)| self.tile(("favorite", i), &f.title, &f.url, true, cx))
            .collect();
        section("Favorites", self.colors.text)
            .child(div().flex().flex_wrap().gap(px(16.0)).children(tiles))
    }

    fn frequent_section(&self, cx: &mut Context<Self>) -> Option<impl IntoElement> {
        let page = &self.settings.start_page;
        let visits: Vec<_> = self
            .history
            .frequent(FREQUENT * 2)
            .unwrap_or_default()
            .into_iter()
            .filter(|v| !page.is_pinned(&v.url))
            .take(FREQUENT)
            .collect();
        if visits.is_empty() {
            return None;
        }
        let tiles: Vec<AnyElement> = visits
            .iter()
            .enumerate()
            .map(|(i, v)| {
                let title = if v.title.is_empty() { &v.url } else { &v.title };
                self.tile(("frequent", i), title, &v.url, false, cx)
            })
            .collect();
        Some(
            section("Frequently Visited", self.colors.text)
                .child(div().flex().flex_wrap().gap(px(16.0)).children(tiles)),
        )
    }

    /// A site: its icon in a rounded square with the name below. Hovering
    /// shows unpin (favorites) or pin (frequent sites).
    fn tile(
        &self,
        id: (&'static str, usize),
        title: &str,
        url: &str,
        pinned: bool,
        cx: &mut Context<Self>,
    ) -> AnyElement {
        let c = self.colors;
        let radius = self.theme.shape.radius_lg;
        let group: SharedString = format!("tile-{}-{}", id.0, id.1).into();
        let (title, url): (SharedString, String) = (short_title(title).into(), url.to_owned());
        let icon = match self.icon_for(&url) {
            Some(image) => div()
                .size(px(30.0))
                .child(img(image).size_full())
                .into_any_element(),
            None => div()
                .text_size(px(24.0))
                .font_weight(FontWeight::SEMIBOLD)
                .text_color(c.muted)
                .child(initial(&title))
                .into_any_element(),
        };
        let open_url = url.clone();
        let (pin_title, pin_url) = (title.to_string(), url.clone());
        div()
            .id(id)
            .group(group.clone())
            .w(px(TILE))
            .flex()
            .flex_col()
            .items_center()
            .gap(px(8.0))
            .cursor_pointer()
            .active(|s| s.opacity(0.75))
            .on_click(cx.listener(move |this, _, _, cx| {
                this.navigate_active(&open_url);
                cx.notify();
            }))
            .child(
                div()
                    .relative()
                    .size(px(TILE_ICON))
                    .flex()
                    .items_center()
                    .justify_center()
                    .rounded(px(radius))
                    .bg(c.capsule)
                    .shadow_sm()
                    .group_hover(group.clone(), |s| s.bg(c.field).shadow_md())
                    .child(icon)
                    .child(
                        div()
                            .id((id.0, id.1 + 10_000))
                            .absolute()
                            .top(px(-6.0))
                            .right(px(-6.0))
                            .size(px(20.0))
                            .flex()
                            .items_center()
                            .justify_center()
                            .rounded_full()
                            .bg(c.capsule)
                            .border_1()
                            .border_color(c.border)
                            .font_family(ICON_FONT)
                            .text_size(px(8.0))
                            .invisible()
                            .group_hover(group, |s| s.visible())
                            .on_mouse_down(
                                MouseButton::Left,
                                cx.listener(move |this, _, _, cx| {
                                    cx.stop_propagation();
                                    this.toggle_pin(&pin_title, &pin_url, cx);
                                }),
                            )
                            .child(if pinned { icon::CLOSE } else { icon::PIN }),
                    ),
            )
            .child(
                div()
                    .w_full()
                    .text_center()
                    .text_size(px(12.0))
                    .text_color(c.text)
                    .truncate()
                    .child(title),
            )
            .into_any_element()
    }

    pub(crate) fn toggle_pin(&mut self, title: &str, url: &str, cx: &mut Context<Self>) {
        let (title, url) = (title.to_owned(), url.to_owned());
        self.update_settings(cx, move |s| {
            s.start_page.toggle_pin(&title, &url);
        });
        cx.notify();
    }

    pub(crate) fn pin_current_page(&mut self, cx: &mut Context<Self>) {
        let Some(tab) = self.active_tab() else {
            return;
        };
        if tab.url == NEW_TAB_URL {
            return;
        }
        let (title, url) = (tab.display_title().to_owned(), tab.url.clone());
        self.toggle_pin(&title, &url, cx);
    }

    fn customize_corner(&self, cx: &mut Context<Self>) -> impl IntoElement {
        let c = self.colors;
        let open = self.customize_open;
        let shown = self
            .springs
            .borrow_mut()
            .drive("customize", 0.0, f32::from(u8::from(open)));
        div()
            .absolute()
            .right(px(20.0))
            .bottom(px(20.0))
            .flex()
            .flex_col()
            .items_end()
            .gap(px(10.0))
            .when(open, |d| d.child(self.customize_panel(shown, cx)))
            .child(
                div()
                    .id("customize")
                    .h(px(30.0))
                    .px(px(12.0))
                    .flex()
                    .items_center()
                    .gap(px(6.0))
                    .rounded_full()
                    .bg(c.capsule)
                    .shadow_sm()
                    .text_size(px(12.0))
                    .hover(move |s| s.bg(c.field))
                    .active(|s| s.opacity(0.7))
                    .on_click(cx.listener(|this, _, _, cx| {
                        this.customize_open = !this.customize_open;
                        cx.notify();
                    }))
                    .child(
                        div()
                            .font_family(ICON_FONT)
                            .text_size(px(11.0))
                            .child(icon::SETTINGS),
                    )
                    .child("Customize"),
            )
    }

    fn customize_panel(&self, shown: f32, cx: &mut Context<Self>) -> impl IntoElement {
        let c = self.colors;
        let page = &self.settings.start_page;
        div()
            .w(px(332.0))
            .p(px(14.0))
            .mb(px(crate::chrome::RISE * (1.0 - shown)))
            .opacity(shown)
            .flex()
            .flex_col()
            .gap(px(12.0))
            .rounded(px(self.theme.shape.radius_lg))
            .bg(c.capsule)
            .shadow_lg()
            .text_size(px(13.0))
            .child(
                check_row("show-favorites", "Favorites", page.show_favorites, c).on_click(
                    cx.listener(|this, _, _, cx| {
                        this.update_settings(cx, |s| {
                            s.start_page.show_favorites = !s.start_page.show_favorites
                        })
                    }),
                ),
            )
            .child(
                check_row("show-frequent", "Frequently Visited", page.show_frequent, c).on_click(
                    cx.listener(|this, _, _, cx| {
                        this.update_settings(cx, |s| {
                            s.start_page.show_frequent = !s.start_page.show_frequent
                        })
                    }),
                ),
            )
            .child(
                div()
                    .text_size(px(12.0))
                    .text_color(c.muted)
                    .child("Background"),
            )
            .child(self.background_swatches(cx))
    }

    fn background_swatches(&self, cx: &mut Context<Self>) -> impl IntoElement {
        let c = self.colors;
        let current = self.settings.start_page.background.clone();
        let swatch = |id: (&'static str, usize), selected: bool| {
            div()
                .id(id)
                .size(px(36.0))
                .rounded(px(8.0))
                .border_2()
                .border_color(if selected { c.accent } else { c.border })
                .cursor_pointer()
                .active(|s| s.opacity(0.7))
        };
        let mut row = div().flex().flex_wrap().gap(px(8.0)).child(
            swatch(("bg", 0), current == StartBackground::Glass)
                .bg(c.glass)
                .on_click(cx.listener(|this, _, _, cx| {
                    this.update_settings(cx, |s| s.start_page.background = StartBackground::Glass)
                })),
        );
        for (i, (from, to, angle)) in GRADIENTS.iter().enumerate() {
            row = row.child(
                swatch(("bg", i + 1), current == StartBackground::Gradient(i))
                    .bg(linear_gradient(
                        *angle,
                        linear_color_stop(gpui::rgb(*from), 0.0),
                        linear_color_stop(gpui::rgb(*to), 1.0),
                    ))
                    .on_click(cx.listener(move |this, _, _, cx| {
                        this.update_settings(cx, move |s| {
                            s.start_page.background = StartBackground::Gradient(i)
                        })
                    })),
            );
        }
        let has_image = matches!(current, StartBackground::Image(_));
        row.child(
            swatch(("bg", 99), has_image)
                .flex()
                .items_center()
                .justify_center()
                .bg(c.field)
                .font_family(ICON_FONT)
                .text_size(px(12.0))
                .text_color(c.muted)
                .on_click(cx.listener(|this, _, _, cx| this.choose_background_image(cx)))
                .child(icon::PICTURE),
        )
    }

    /// Asks for a picture, copies it into the data folder (so moving or
    /// deleting the original does not break the page), and uses it.
    fn choose_background_image(&mut self, cx: &mut Context<Self>) {
        let picked = cx.prompt_for_paths(PathPromptOptions {
            files: true,
            directories: false,
            multiple: false,
            prompt: Some("Choose a start page background".into()),
        });
        let root = self.paths.root().to_owned();
        cx.spawn(async move |this, cx| {
            let Ok(Ok(Some(paths))) = picked.await else {
                return;
            };
            let Some(source) = paths.into_iter().next() else {
                return;
            };
            let ext = source
                .extension()
                .and_then(|e| e.to_str())
                .unwrap_or("png")
                .to_ascii_lowercase();
            let target = root.join(format!("start-background.{ext}"));
            if let Err(e) = std::fs::copy(&source, &target) {
                return eprintln!("background copy failed: {e}");
            }
            let path = target.display().to_string();
            let _ = this.update(cx, move |b, cx| {
                b.update_settings(cx, move |s| {
                    s.start_page.background = StartBackground::Image(path)
                });
            });
        })
        .detach();
    }
}

fn section(title: &'static str, color: gpui::Hsla) -> gpui::Div {
    div().flex().flex_col().gap(px(14.0)).child(
        div()
            .text_size(px(17.0))
            .font_weight(FontWeight::SEMIBOLD)
            .text_color(color)
            .child(title),
    )
}

fn check_row(
    id: &'static str,
    label: &'static str,
    on: bool,
    c: crate::style::ChromeColors,
) -> gpui::Stateful<gpui::Div> {
    div()
        .id(id)
        .flex()
        .items_center()
        .gap(px(10.0))
        .cursor_pointer()
        .child(
            div()
                .size(px(16.0))
                .flex()
                .items_center()
                .justify_center()
                .rounded(px(4.0))
                .border_1()
                .border_color(if on { c.accent } else { c.border })
                .bg(if on { c.accent } else { c.field })
                .font_family(ICON_FONT)
                .text_size(px(9.0))
                .text_color(c.on_accent)
                .when(on, |d| d.child(icon::CHECK)),
        )
        .child(label)
}

/// "GitHub · Change is constant…" reads better on a tile as "GitHub".
fn short_title(title: &str) -> String {
    let first = title
        .split([' ', ',', ':', '-', '|', '\u{00b7}', '\u{2013}', '\u{2014}'])
        .find(|p| !p.is_empty());
    let short = if title.chars().count() > 18 {
        first.unwrap_or(title)
    } else {
        title
    };
    short.to_owned()
}

fn initial(title: &str) -> String {
    title
        .chars()
        .find(|c| c.is_alphanumeric())
        .map(|c| c.to_uppercase().to_string())
        .unwrap_or_default()
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn short_titles() {
        assert_eq!(short_title("GitHub"), "GitHub");
        assert_eq!(
            short_title("GitHub \u{00b7} Change is constant. GitHub keeps you ahead."),
            "GitHub"
        );
        assert_eq!(short_title("Wikipedia, the free encyclopedia"), "Wikipedia");
        assert_eq!(initial("  wikipedia"), "W");
    }
}
