//! Extensions: adding unpacked folders or .zip files, keeping every
//! workspace's profile in line with the settings list, and the settings pane.

use std::fs;
use std::io;
use std::path::{Path, PathBuf};

use browser_core::settings::ExtensionEntry;
use browser_engine::ExtensionInfo;
use gpui::{
    AnyElement, Context, FontWeight, IntoElement, ParentElement, PathPromptOptions, SharedString,
    StatefulInteractiveElement, Styled, div, prelude::*, px,
};

use crate::browser::Browser;

/// uBlock Origin Lite: a well-known content blocker that works as an
/// unpacked MV3 extension, including on video sites.
const UBOL_RELEASES: &str = "https://github.com/uBlockOrigin/uBOL-home/releases/latest";

/// Unpacking stops past this, so a hostile archive cannot fill the disk.
const MAX_UNPACKED: u64 = 200 * 1024 * 1024;

pub(crate) enum ExtensionResult {
    Installed {
        folder: PathBuf,
        info: ExtensionInfo,
    },
    Failed {
        folder: PathBuf,
        error: String,
    },
}

impl Browser {
    /// Loads what settings list but the profile lacks, removes what it has
    /// but settings dropped, and matches enabled state. Once per profile.
    pub(crate) fn sync_extensions(&mut self) {
        if self.extensions_synced {
            return;
        }
        if self.any_view().is_none() {
            return;
        }
        self.extensions_synced = true;
        let wanted = self.settings.extensions.clone();
        let inbox = self.extension_inbox.clone();
        let wake = self.wake_handle();
        let Some(view) = self.any_view() else {
            return;
        };
        let listed = view.extensions(move |installed| {
            let mut results = inbox.borrow_mut();
            for entry in &wanted {
                let found = installed
                    .iter()
                    .find(|i| !entry.id.is_empty() && i.id == entry.id);
                match found {
                    Some(info) => results.push(ExtensionResult::Installed {
                        folder: entry.folder.clone(),
                        info: info.clone(),
                    }),
                    None => results.push(ExtensionResult::Failed {
                        folder: entry.folder.clone(),
                        error: String::new(), // not loaded yet; installed below
                    }),
                }
            }
            drop(results);
            let _ = wake.unbounded_send(());
        });
        if let Err(e) = listed {
            self.extension_status = Some(format!("Extensions are not available: {e}"));
        }
    }

    /// Applies install results; an empty error means "load it now".
    pub(crate) fn apply_extension_results(&mut self) {
        let results: Vec<ExtensionResult> = self.extension_inbox.borrow_mut().drain(..).collect();
        let mut changed = false;
        for result in results {
            match result {
                ExtensionResult::Installed { folder, info } => {
                    let enabled = self.entry_mut(&folder).map(|entry| {
                        entry.id = info.id.clone();
                        entry.name = info.name.clone();
                        entry.enabled
                    });
                    if let (Some(enabled), Some(view)) = (enabled, self.any_view())
                        && enabled != info.enabled
                    {
                        let _ = view.set_extension_enabled(&info.id, enabled);
                    }
                    self.extension_status = None;
                    changed = true;
                }
                ExtensionResult::Failed { folder, error } if error.is_empty() => {
                    self.load_extension(folder);
                }
                ExtensionResult::Failed { folder, error } => {
                    self.extension_status =
                        Some(format!("Could not load {}: {error}", folder.display()));
                }
            }
        }
        if changed {
            let _ = self.settings.save(self.kv());
        }
    }

    fn entry_mut(&mut self, folder: &Path) -> Option<&mut ExtensionEntry> {
        self.settings
            .extensions
            .iter_mut()
            .find(|e| e.folder == folder)
    }

    fn load_extension(&mut self, folder: PathBuf) {
        let Some(view) = self.any_view() else {
            return; // loaded by the next sync, once a page is open
        };
        let (inbox, wake) = (self.extension_inbox.clone(), self.wake_handle());
        let target = folder.clone();
        let started = view.install_extension(&folder, move |result| {
            inbox.borrow_mut().push(match result {
                Ok(info) => ExtensionResult::Installed {
                    folder: target,
                    info,
                },
                Err(error) => ExtensionResult::Failed {
                    folder: target,
                    error,
                },
            });
            let _ = wake.unbounded_send(());
        });
        if let Err(e) = started {
            self.extension_status = Some(format!("Extensions are not available: {e}"));
        }
    }

    fn add_extension(&mut self, folder: PathBuf, cx: &mut Context<Self>) {
        if !folder.join("manifest.json").is_file() {
            self.extension_status = Some(
                "That folder has no manifest.json, so it is not an unpacked extension.".into(),
            );
            return cx.notify();
        }
        if self.entry_mut(&folder).is_none() {
            self.settings.extensions.push(ExtensionEntry {
                folder: folder.clone(),
                id: String::new(),
                name: String::new(),
                enabled: true,
            });
            let _ = self.settings.save(self.kv());
        }
        if self.any_view().is_none() {
            self.extension_status = Some("Added. It loads as soon as a web page is open.".into());
        }
        self.load_extension(folder);
        cx.notify();
    }

    fn pick_extension(&mut self, zip: bool, cx: &mut Context<Self>) {
        let picked = cx.prompt_for_paths(PathPromptOptions {
            files: zip,
            directories: !zip,
            multiple: false,
            prompt: Some(
                if zip {
                    "Add extension (.zip)"
                } else {
                    "Add unpacked extension"
                }
                .into(),
            ),
        });
        let target_root = self.paths.extensions();
        cx.spawn(async move |this, cx| {
            let Ok(Ok(Some(paths))) = picked.await else {
                return;
            };
            let Some(path) = paths.into_iter().next() else {
                return;
            };
            let folder = if zip {
                unpack_extension(&path, &target_root).map_err(|e| e.to_string())
            } else {
                Ok(path)
            };
            let _ = this.update(cx, move |b, cx| match folder {
                Ok(folder) => b.add_extension(folder, cx),
                Err(e) => {
                    b.extension_status = Some(format!("Could not unpack the .zip: {e}"));
                    cx.notify();
                }
            });
        })
        .detach();
    }

    fn toggle_extension(&mut self, folder: &Path, cx: &mut Context<Self>) {
        let Some(entry) = self.entry_mut(folder) else {
            return;
        };
        entry.enabled = !entry.enabled;
        let (id, enabled) = (entry.id.clone(), entry.enabled);
        let _ = self.settings.save(self.kv());
        if let (false, Some(view)) = (id.is_empty(), self.any_view()) {
            let _ = view.set_extension_enabled(&id, enabled);
        }
        cx.notify();
    }

    /// Opens the extension's own page (its settings or its toolbar popup)
    /// in a tab; WebView2 has no extension toolbar to anchor a popup to.
    fn open_extension_page(&mut self, folder: &Path, popup: bool, cx: &mut Context<Self>) {
        let Some(entry) = self.settings.extensions.iter().find(|e| e.folder == folder) else {
            return;
        };
        if entry.id.is_empty() {
            self.extension_status =
                Some("This extension has not loaded yet; open any web page first.".into());
            return cx.notify();
        }
        let Some(page) = manifest_page(folder, popup) else {
            let what = if popup { "a popup" } else { "a settings page" };
            self.extension_status = Some(format!("This extension has no {what}."));
            return cx.notify();
        };
        let url = format!(
            "chrome-extension://{}/{}",
            entry.id,
            page.trim_start_matches('/')
        );
        self.close_settings(cx);
        self.open_tab(&url, None, true);
        cx.notify();
    }

    fn remove_extension(&mut self, folder: &Path, cx: &mut Context<Self>) {
        let Some(i) = self
            .settings
            .extensions
            .iter()
            .position(|e| e.folder == folder)
        else {
            return;
        };
        let entry = self.settings.extensions.remove(i);
        let _ = self.settings.save(self.kv());
        if let (false, Some(view)) = (entry.id.is_empty(), self.any_view()) {
            let _ = view.remove_extension(&entry.id);
        }
        // Only our own unpacked copies are deleted; a user's folder stays.
        if entry.folder.starts_with(self.paths.extensions()) {
            let _ = fs::remove_dir_all(&entry.folder);
        }
        cx.notify();
    }

    pub(crate) fn extensions_section(&self, cx: &mut Context<Self>) -> AnyElement {
        let c = self.colors;
        let entries = self.settings.extensions.clone();
        let list =
            div()
                .flex()
                .flex_col()
                .gap(px(10.0))
                .children(entries.into_iter().enumerate().map(|(i, e)| {
                    let name: SharedString = if e.name.is_empty() {
                        e.folder
                            .file_name()
                            .map(|n| n.to_string_lossy().into_owned())
                            .unwrap_or_default()
                            .into()
                    } else {
                        e.name.clone().into()
                    };
                    let path: SharedString = e.folder.display().to_string().into();
                    let (toggle_folder, remove_folder) = (e.folder.clone(), e.folder.clone());
                    let (options_folder, popup_folder) = (e.folder.clone(), e.folder.clone());
                    div()
                        .flex()
                        .items_center()
                        .gap(px(12.0))
                        .child(
                            div()
                                .flex_1()
                                .min_w_0()
                                .flex()
                                .flex_col()
                                .gap(px(2.0))
                                .child(div().font_weight(FontWeight::MEDIUM).truncate().child(name))
                                .child(
                                    div()
                                        .text_size(px(12.0))
                                        .text_color(c.muted)
                                        .truncate()
                                        .child(path),
                                ),
                        )
                        .child(
                            self.action_button(("ext-options", i), "Settings", false, cx)
                                .on_click(cx.listener(move |this, _, _, cx| {
                                    this.open_extension_page(&options_folder, false, cx)
                                })),
                        )
                        .child(
                            self.action_button(("ext-popup", i), "Popup", false, cx)
                                .on_click(cx.listener(move |this, _, _, cx| {
                                    this.open_extension_page(&popup_folder, true, cx)
                                })),
                        )
                        .child(
                            self.action_button(("ext-remove", i), "Remove", false, cx)
                                .on_click(cx.listener(move |this, _, _, cx| {
                                    this.remove_extension(&remove_folder, cx)
                                })),
                        )
                        .child(
                            crate::settings_ui::toggle_for(
                                ("ext-toggle", i),
                                e.enabled,
                                c,
                                &self.springs,
                            )
                            .on_click(cx.listener(
                                move |this, _, _, cx| this.toggle_extension(&toggle_folder, cx),
                            )),
                        )
                }));
        div()
            .flex()
            .flex_col()
            .gap(px(16.0))
            .child(
                div()
                    .text_size(px(12.0))
                    .text_color(c.muted)
                    .child("Extensions run inside every page they match and can read what you see there. Only add ones you trust."),
            )
            .child(
                div()
                    .flex()
                    .gap(px(8.0))
                    .child(self.action_button("ext-add-folder", "Add folder…", false, cx).on_click(cx.listener(
                        |this, _, _, cx| this.pick_extension(false, cx),
                    )))
                    .child(self.action_button("ext-add-zip", "Add .zip…", false, cx).on_click(cx.listener(
                        |this, _, _, cx| this.pick_extension(true, cx),
                    )))
                    .child(div().flex_1())
                    .child(self.action_button("ext-ubol", "Get uBlock Origin Lite", false, cx).on_click(cx.listener(
                        |this, _, _, cx| {
                            this.close_settings(cx);
                            this.open_tab(UBOL_RELEASES, None, true);
                        },
                    ))),
            )
            .children(self.extension_status.clone().map(|s| {
                div().text_size(px(12.0)).text_color(c.accent).child(SharedString::from(s))
            }))
            .when(self.settings.extensions.is_empty(), |d| {
                d.child(
                    div()
                        .text_size(px(12.0))
                        .text_color(c.muted)
                        .child("For YouTube ads: download the uBOLite …chromium.zip from the releases page, then use Add .zip…"),
                )
            })
            .child(list)
            .into_any_element()
    }
}

/// Unpacks `zip` under `root/<name>` and returns the folder holding
/// manifest.json. Entries that would escape the folder are refused.
fn unpack_extension(zip: &Path, root: &Path) -> io::Result<PathBuf> {
    let stem = zip
        .file_stem()
        .map(|s| s.to_string_lossy().into_owned())
        .unwrap_or_else(|| "extension".to_owned());
    let target = root.join(sanitize(&stem));
    if target.exists() {
        fs::remove_dir_all(&target)?;
    }
    fs::create_dir_all(&target)?;
    let mut archive = zip::ZipArchive::new(fs::File::open(zip)?).map_err(io::Error::other)?;
    let mut written = 0u64;
    for i in 0..archive.len() {
        let mut file = archive.by_index(i).map_err(io::Error::other)?;
        let Some(relative) = file.enclosed_name() else {
            return Err(io::Error::other("the archive has an unsafe path"));
        };
        let out = target.join(relative);
        if file.is_dir() {
            fs::create_dir_all(&out)?;
            continue;
        }
        if let Some(parent) = out.parent() {
            fs::create_dir_all(parent)?;
        }
        written += file.size();
        if written > MAX_UNPACKED {
            return Err(io::Error::other("the archive is too large"));
        }
        io::copy(&mut file, &mut fs::File::create(&out)?)?;
    }
    manifest_folder(&target).ok_or_else(|| io::Error::other("no manifest.json inside"))
}

/// `options_ui.page` / `options_page`, or the toolbar popup, from manifest.json.
fn manifest_page(folder: &Path, popup: bool) -> Option<String> {
    let text = fs::read_to_string(folder.join("manifest.json")).ok()?;
    let manifest: serde_json::Value =
        serde_json::from_str(text.trim_start_matches('\u{feff}')).ok()?;
    let page = if popup {
        manifest["action"]["default_popup"]
            .as_str()
            .or(manifest["browser_action"]["default_popup"].as_str())
    } else {
        manifest["options_ui"]["page"]
            .as_str()
            .or(manifest["options_page"].as_str())
    };
    // A page is a path inside the extension, never a URL elsewhere.
    page.filter(|p| !p.contains(':') && !p.contains(".."))
        .map(str::to_owned)
}

/// The shallowest folder (up to two levels) that holds manifest.json.
fn manifest_folder(root: &Path) -> Option<PathBuf> {
    if root.join("manifest.json").is_file() {
        return Some(root.to_path_buf());
    }
    let children = fs::read_dir(root).ok()?;
    children
        .filter_map(Result::ok)
        .map(|e| e.path())
        .filter(|p| p.is_dir())
        .find(|p| p.join("manifest.json").is_file())
}

fn sanitize(name: &str) -> String {
    let safe: String = name
        .chars()
        .map(|c| {
            if c.is_ascii_alphanumeric() || c == '-' || c == '_' || c == '.' {
                c
            } else {
                '_'
            }
        })
        .collect();
    // "." or ".." would point at the extensions folder or its parent.
    if safe.chars().all(|c| c == '.') {
        return "extension".to_owned();
    }
    safe
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn finds_manifest_in_root_or_one_folder_down() -> io::Result<()> {
        let dir = std::env::temp_dir().join(format!("umbrella-ext-{}", std::process::id()));
        fs::create_dir_all(dir.join("inner"))?;
        fs::write(dir.join("inner").join("manifest.json"), "{}")?;
        assert_eq!(manifest_folder(&dir), Some(dir.join("inner")));
        fs::write(dir.join("manifest.json"), "{}")?;
        assert_eq!(manifest_folder(&dir), Some(dir.clone()));
        fs::remove_dir_all(&dir)?;
        Ok(())
    }

    #[test]
    fn names_are_safe_folder_names() {
        assert_eq!(
            sanitize("uBOLite_2025.1.chromium.mv3"),
            "uBOLite_2025.1.chromium.mv3"
        );
        assert_eq!(sanitize("../evil name"), ".._evil_name");
        assert_eq!(sanitize(".."), "extension");
    }
}
