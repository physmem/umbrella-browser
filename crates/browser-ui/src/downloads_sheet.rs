//! The downloads sheet and the download flow around it: save-as prompts,
//! opening safe files when done, cancelling.

use std::path::PathBuf;

use browser_core::TabId;
use browser_engine::DownloadId;
use gpui::{
    AnyElement, Context, FontWeight, InteractiveElement, IntoElement, ParentElement, SharedString,
    StatefulInteractiveElement, Styled, Window, div, prelude::*, px,
};

use crate::browser::Browser;
use crate::downloads_ui::{Download, DownloadState};
use crate::style::{ICON_FONT, icon};

const SHEET_WIDTH: f32 = 560.0;

impl Browser {
    pub(crate) fn open_downloads(&mut self, cx: &mut Context<Self>) {
        if self.downloads_open {
            return self.close_downloads(cx);
        }
        self.settings_open = false;
        self.omnibox_editing = false;
        self.suggestions.clear();
        self.page_info_open = false;
        self.downloads_open = true;
        self.capture_page_preview();
        self.sync_views();
        // Esc must reach the browser root, not the hidden page.
        self.defer_in_window(cx, |b, window, cx| {
            crate::platform::take_focus(b.hwnd);
            window.focus(&b.focus, cx);
        });
        cx.notify();
    }

    pub(crate) fn close_downloads(&mut self, cx: &mut Context<Self>) {
        if !self.downloads_open {
            return;
        }
        self.downloads_open = false;
        self.sync_views();
        self.focus_page();
        cx.notify();
    }

    /// "Ask where to save" is on: show the save dialog, then let the
    /// download continue to the chosen path or cancel it.
    pub(crate) fn ask_save_path(
        &mut self,
        tab: TabId,
        download: DownloadId,
        suggested: PathBuf,
        cx: &mut Context<Self>,
    ) {
        let folder = suggested.parent().map(PathBuf::from).unwrap_or_default();
        let name = suggested
            .file_name()
            .map(|n| n.to_string_lossy().into_owned());
        let picked = cx.prompt_for_new_path(&folder, name.as_deref());
        cx.spawn(async move |this, cx| {
            let path = picked.await.ok().and_then(Result::ok).flatten();
            let _ = this.update(cx, move |b, _| b.resolve_download(tab, download, path));
        })
        .detach();
    }

    fn resolve_download(&mut self, tab: TabId, download: DownloadId, path: Option<PathBuf>) {
        let Some(view) = self.view(tab) else {
            return; // the tab closed; WebView2 dropped the download with it
        };
        if let Err(e) = view.resolve_download(download, path) {
            eprintln!("download {download:?}: {e}");
        }
    }

    pub(crate) fn on_download_finished(
        &mut self,
        download: DownloadId,
        ok: bool,
        cx: &mut Context<Self>,
    ) {
        let open = self.settings.downloads.open_when_done;
        let Some(done) = self.downloads.finished(download, ok) else {
            return;
        };
        if ok && open && done.safe_to_open() {
            cx.open_with_system(&done.file);
        }
    }

    fn cancel_download(&self, d: &Download) {
        if let Some(view) = self.view(d.tab) {
            let _ = view.cancel_download(d.id);
        }
    }

    pub(crate) fn downloads_sheet(
        &self,
        window: &Window,
        cx: &mut Context<Self>,
    ) -> Option<AnyElement> {
        let reveal = self.springs.borrow_mut().drive(
            "downloads",
            0.0,
            f32::from(u8::from(self.downloads_open)),
        );
        if !self.downloads_open {
            return None;
        }
        let c = self.colors;
        let height =
            (f32::from(window.viewport_size().height) - crate::style::CHROME_HEIGHT - 32.0)
                .max(120.0);
        let items = self.downloads.items();
        let finished = items.iter().any(|d| d.state != DownloadState::Active);
        let card = div()
            .id("downloads-sheet")
            .mt(px(crate::chrome::RISE * 2.0 * (1.0 - reveal)))
            .opacity(reveal)
            .w(px(SHEET_WIDTH))
            .max_h(px(height))
            .overflow_y_scroll()
            .p(px(16.0))
            .flex()
            .flex_col()
            .gap(px(4.0))
            .rounded(px(self.theme.shape.radius_lg))
            .bg(c.capsule)
            .shadow_lg()
            .text_size(px(13.0))
            .child(
                div()
                    .flex()
                    .items_center()
                    .gap(px(8.0))
                    .pb(px(8.0))
                    .child(
                        div()
                            .flex_1()
                            .text_size(px(16.0))
                            .font_weight(FontWeight::SEMIBOLD)
                            .child("Downloads"),
                    )
                    .when(finished, |d| {
                        d.child(small_action("clear-downloads", "Clear list", c).on_click(
                            cx.listener(|this, _, _, cx| {
                                this.downloads.clear_finished();
                                cx.notify();
                            }),
                        ))
                    })
                    .child(small_action("downloads-folder", "Open folder", c).on_click(
                        cx.listener(|this, _, _, cx| {
                            let folder = this.download_folder();
                            cx.open_with_system(&folder);
                        }),
                    ))
                    .child(
                        icon_action("downloads-close", icon::CLOSE, c)
                            .on_click(cx.listener(|this, _, _, cx| this.close_downloads(cx))),
                    ),
            )
            .when(items.is_empty(), |d| {
                d.child(
                    div()
                        .py(px(24.0))
                        .flex()
                        .justify_center()
                        .text_color(c.muted)
                        .child("Files you download show up here."),
                )
            })
            .children(
                items
                    .iter()
                    .enumerate()
                    .map(|(i, d)| self.download_row(i, d, cx)),
            );
        Some(self.sheet_backdrop(card.into_any_element()))
    }

    /// The folder new downloads go to, for "Open folder".
    fn download_folder(&self) -> PathBuf {
        if let Some(folder) = &self.settings.downloads.folder {
            return folder.clone();
        }
        self.downloads
            .items()
            .first()
            .and_then(|d| d.file.parent().map(PathBuf::from))
            .or_else(|| std::env::var_os("USERPROFILE").map(|h| PathBuf::from(h).join("Downloads")))
            .unwrap_or_default()
    }

    fn download_row(&self, i: usize, d: &Download, cx: &mut Context<Self>) -> AnyElement {
        let c = self.colors;
        let name: SharedString = d.file_name().into();
        let status: SharedString = d.status_label().into();
        let (id, file) = (d.id, d.file.clone());
        let fraction = d.fraction();
        let active = d.state == DownloadState::Active;
        let done = d.state == DownloadState::Done;
        let width = self.springs.borrow_mut().drive_keyed(
            format!("download-{}", id.0),
            0.0,
            fraction.unwrap_or(0.0),
        );
        div()
            .id(("download", i))
            .p(px(8.0))
            .flex()
            .items_center()
            .gap(px(10.0))
            .rounded(px(self.theme.shape.radius_md))
            .hover(move |s| s.bg(c.hover))
            .when(done, |row| {
                let file = file.clone();
                row.cursor_pointer()
                    .on_click(cx.listener(move |_, _, _, cx| cx.open_with_system(&file)))
            })
            .child(
                div()
                    .size(px(32.0))
                    .flex_none()
                    .flex()
                    .items_center()
                    .justify_center()
                    .rounded(px(8.0))
                    .bg(c.field)
                    .font_family(ICON_FONT)
                    .text_size(px(13.0))
                    .text_color(if d.state == DownloadState::Failed {
                        c.muted
                    } else {
                        c.text
                    })
                    .child(icon::DOWNLOAD),
            )
            .child(
                div()
                    .flex_1()
                    .min_w_0()
                    .flex()
                    .flex_col()
                    .gap(px(4.0))
                    .child(div().truncate().font_weight(FontWeight::MEDIUM).child(name))
                    .when(active && fraction.is_some(), |col| {
                        col.child(
                            div().h(px(4.0)).w_full().rounded_full().bg(c.field).child(
                                div()
                                    .h_full()
                                    .w(gpui::relative(width))
                                    .rounded_full()
                                    .bg(c.accent),
                            ),
                        )
                    })
                    .child(div().text_size(px(12.0)).text_color(c.muted).child(status)),
            )
            .when(active, |row| {
                let item = d.clone();
                row.child(
                    small_action(("cancel", i), "Cancel", c)
                        .on_click(cx.listener(move |this, _, _, _| this.cancel_download(&item))),
                )
            })
            .when(!active, |row| {
                let item = d.clone();
                row.child(
                    small_action(("reveal", i), "Show in folder", c)
                        .on_click(cx.listener(move |_, _, _, _| item.reveal())),
                )
            })
            .into_any_element()
    }
}

fn small_action(
    id: impl Into<gpui::ElementId>,
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

fn icon_action(
    id: &'static str,
    glyph: &'static str,
    c: crate::style::ChromeColors,
) -> gpui::Stateful<gpui::Div> {
    div()
        .id(id)
        .size(px(28.0))
        .flex()
        .items_center()
        .justify_center()
        .rounded(px(6.0))
        .font_family(ICON_FONT)
        .text_size(px(10.0))
        .hover(move |s| s.bg(c.hover))
        .child(glyph)
}
