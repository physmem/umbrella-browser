//! Import from other browsers: bookmarks from a Chromium profile, passwords
//! from an exported CSV into the vault.

use std::path::PathBuf;

use browser_core::import::{self, ImportedLogin};
use browser_core::vault::{Credential, Origin};
use browser_engine::{ImportCookie, SameSite};
use gpui::{
    AnyElement, Context, IntoElement, ParentElement, PathPromptOptions, SharedString,
    StatefulInteractiveElement, Styled, div, px,
};
use serde::Deserialize;

use browser_core::TabId;

use crate::browser::{Browser, NEW_TAB_URL};

impl Browser {
    fn import_bookmarks(&mut self, name: &'static str, file: PathBuf, cx: &mut Context<Self>) {
        let json = match std::fs::read_to_string(&file) {
            Ok(json) => json,
            Err(e) => {
                self.import_status = Some(format!("Could not read {name}'s bookmarks: {e}"));
                return cx.notify();
            }
        };
        let now = crate::browser::unix_now();
        let marks = import::chromium_bookmarks(&json);
        let added = marks
            .iter()
            .filter(|m| self.bookmarks.add(&m.url, &m.title, now).unwrap_or(false))
            .count();
        self.import_status = Some(format!(
            "Imported {added} bookmarks from {name} ({} were already here).",
            marks.len() - added
        ));
        cx.notify();
    }

    fn pick_password_csv(&mut self, cx: &mut Context<Self>) {
        let picked = cx.prompt_for_paths(PathPromptOptions {
            files: true,
            directories: false,
            multiple: false,
            prompt: Some("Import passwords (.csv)".into()),
        });
        cx.spawn(async move |this, cx| {
            let Ok(Ok(Some(paths))) = picked.await else {
                return;
            };
            let Some(path) = paths.into_iter().next() else {
                return;
            };
            let parsed = std::fs::read_to_string(&path)
                .map_err(|e| e.to_string())
                .and_then(|text| import::password_csv(&zeroize::Zeroizing::new(text)));
            let _ = this.update(cx, move |b, cx| {
                match parsed {
                    Ok(logins) => b.import_logins(logins, cx),
                    Err(e) => b.import_status = Some(format!("Could not import that file: {e}")),
                }
                cx.notify();
            });
        })
        .detach();
    }

    fn import_logins(&mut self, logins: Vec<ImportedLogin>, cx: &mut Context<Self>) {
        let Some(vault) = self.vault.unlocked() else {
            self.import_status = Some(
                "Unlock the vault with the key prompt in the toolbar, then import the file again."
                    .into(),
            );
            self.request_unlock(cx);
            return;
        };
        let (mut saved, mut skipped) = (0, 0);
        for login in &logins {
            let Ok(origin) = Origin::parse(&login.url) else {
                skipped += 1;
                continue;
            };
            let credential = Credential::new(&login.username, &login.password);
            match vault.save(&origin, &credential) {
                Ok(_) => saved += 1,
                Err(_) => skipped += 1,
            }
        }
        self.import_status = Some(format!(
            "Imported {saved} passwords{}. Delete the CSV now: it holds them in plain text.",
            if skipped > 0 {
                format!(", skipped {skipped}")
            } else {
                String::new()
            }
        ));
    }

    fn pick_cookie_file(&mut self, cx: &mut Context<Self>) {
        let picked = cx.prompt_for_paths(PathPromptOptions {
            files: true,
            directories: false,
            multiple: false,
            prompt: Some("Import cookies (.json)".into()),
        });
        cx.spawn(async move |this, cx| {
            let Ok(Ok(Some(paths))) = picked.await else {
                return;
            };
            let Some(path) = paths.into_iter().next() else {
                return;
            };
            let parsed = std::fs::read_to_string(&path)
                .map_err(|e| e.to_string())
                .and_then(|text| parse_cookie_json(&text));
            let _ = this.update(cx, move |b, cx| {
                match parsed {
                    Ok(cookies) => b.apply_cookies(cookies, cx),
                    Err(e) => b.import_status = Some(format!("Could not read that file: {e}")),
                }
                cx.notify();
            });
        })
        .detach();
    }

    fn apply_cookies(&mut self, cookies: Vec<ImportCookie>, cx: &mut Context<Self>) {
        if cookies.is_empty() {
            self.import_status = Some("That file had no cookies.".into());
            return;
        }
        // The site most of the cookies belong to, to open signed in afterwards.
        let host = cookies
            .iter()
            .map(|c| c.domain.trim_start_matches('.').to_owned())
            .max_by_key(|h| {
                cookies
                    .iter()
                    .filter(|c| c.domain.trim_start_matches('.') == *h)
                    .count()
            });
        self.close_settings(cx);
        // Set the cookies before the site loads, so a logged-out page cannot
        // re-set the old session cookies first. If a page is already live, use
        // its profile-wide cookie manager and open the site once done;
        // otherwise open the site's tab and set them when it goes live.
        if self.any_view().is_some() {
            self.pending_cookie_site = host;
            if let Some(view) = self.any_view() {
                let _ = view.import_cookies(cookies);
            }
        } else {
            let url = host
                .as_deref()
                .map(|h| format!("https://{h}/"))
                .unwrap_or_else(|| NEW_TAB_URL.to_owned());
            self.open_tab(&url, None, true);
            if let Some(id) = self.active_id() {
                self.pending_cookies = Some((id, cookies));
            }
        }
        self.import_status = Some("Importing cookies.".into());
    }

    /// Starts the cookie import for tab `id` once its page is live. The
    /// engine clears the sites, sets the cookies and reports back on
    /// `on_cookies_imported`, which reloads the page signed in.
    pub(crate) fn flush_pending_cookies(&mut self, id: TabId, cx: &mut Context<Self>) {
        if self.pending_cookies.as_ref().is_none_or(|(t, _)| *t != id) {
            return;
        }
        let Some((_, cookies)) = self.pending_cookies.take() else {
            return;
        };
        if let Some(view) = self.view(id) {
            let _ = view.import_cookies(cookies);
        }
        cx.notify();
    }

    pub(crate) fn on_cookies_imported(&mut self, id: TabId, count: usize, cx: &mut Context<Self>) {
        match self.pending_cookie_site.take() {
            // Cookies set on an existing page's profile: open the site fresh.
            Some(host) => self.open_tab(&format!("https://{host}/"), None, true),
            // Cookies set on the site's own tab: reload it signed in.
            None => {
                if let Some(view) = self.view(id) {
                    let _ = view.reload();
                }
            }
        }
        self.import_status = Some(format!("Imported {count} cookies."));
        cx.notify();
    }

    pub(crate) fn import_section(&self, passwords: bool, cx: &mut Context<Self>) -> AnyElement {
        let c = self.colors;
        let browsers = std::env::var_os("LOCALAPPDATA")
            .map(|d| import::installed_browsers(&PathBuf::from(d)))
            .unwrap_or_default();
        let buttons = div().flex().flex_wrap().gap(px(8.0));
        let buttons = if passwords {
            buttons.child(
                self.action_button("import-csv", "Import passwords from CSV…", false, cx)
                    .on_click(cx.listener(|this, _, _, cx| this.pick_password_csv(cx))),
            )
        } else {
            buttons
                .children(browsers.into_iter().enumerate().map(|(i, b)| {
                    let (name, file) = (b.name, b.bookmarks.clone());
                    self.action_button(
                        ("import-bookmarks", i),
                        format!("Bookmarks from {name}"),
                        false,
                        cx,
                    )
                    .on_click(cx.listener(move |this, _, _, cx| {
                        this.import_bookmarks(name, file.clone(), cx)
                    }))
                }))
                .child(
                    self.action_button(
                        "import-cookies",
                        "Import cookies (.json)\u{2026}",
                        false,
                        cx,
                    )
                    .on_click(cx.listener(|this, _, _, cx| this.pick_cookie_file(cx))),
                )
        };
        let hint = if passwords {
            "In Chrome, Edge or Brave: Settings > Passwords > Export passwords. Then pick that file here."
        } else {
            "Copy bookmarks from another browser, or import a cookie export (Cookie-Editor JSON) to carry a signed-in session over."
        };
        div()
            .flex()
            .flex_col()
            .gap(px(8.0))
            .child(div().text_size(px(12.0)).text_color(c.muted).child(hint))
            .child(buttons)
            .children(self.import_status.clone().map(|s| {
                div()
                    .text_size(px(12.0))
                    .text_color(c.accent)
                    .child(SharedString::from(s))
            }))
            .into_any_element()
    }
}

/// Cookie-Editor's JSON export: an array of cookie objects.
#[derive(Deserialize)]
struct ExportedCookie {
    name: String,
    value: String,
    domain: String,
    #[serde(default = "root_path")]
    path: String,
    #[serde(default)]
    secure: bool,
    #[serde(default, rename = "httpOnly")]
    http_only: bool,
    #[serde(default, rename = "sameSite")]
    same_site: Option<String>,
    #[serde(default, rename = "expirationDate")]
    expiration: Option<f64>,
    #[serde(default)]
    session: bool,
}

fn root_path() -> String {
    "/".to_owned()
}

fn parse_cookie_json(text: &str) -> Result<Vec<ImportCookie>, String> {
    let raw: Vec<ExportedCookie> = serde_json::from_str(text.trim_start_matches('\u{feff}'))
        .map_err(|_| "that is not a Cookie-Editor JSON export".to_owned())?;
    Ok(raw
        .into_iter()
        .filter(|c| !c.name.is_empty() && !c.domain.is_empty())
        .map(|c| ImportCookie {
            same_site: same_site(c.same_site.as_deref()),
            // A session cookie has no expiry even if the file lists a date.
            expires: (!c.session).then_some(()).and(c.expiration),
            name: c.name,
            value: c.value,
            domain: c.domain,
            path: c.path,
            secure: c.secure,
            http_only: c.http_only,
        })
        .collect())
}

fn same_site(kind: Option<&str>) -> SameSite {
    match kind.unwrap_or("").to_ascii_lowercase().as_str() {
        "no_restriction" | "none" => SameSite::None,
        "strict" => SameSite::Strict,
        // "lax", "unspecified", or missing: the browser default.
        _ => SameSite::Lax,
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn parses_cookie_editor_json() {
        let json = r#"[
            {"domain":".netflix.com","name":"NetflixId","value":"abc","path":"/","secure":true,"httpOnly":true,"sameSite":"lax","expirationDate":1810687571.0,"session":false},
            {"domain":".netflix.com","name":"s","value":"x","sameSite":"no_restriction","session":true}
        ]"#;
        let cookies = parse_cookie_json(json).unwrap();
        assert_eq!(cookies.len(), 2);
        assert_eq!(cookies[0].domain, ".netflix.com");
        assert_eq!(cookies[0].same_site, SameSite::Lax);
        assert_eq!(cookies[0].expires, Some(1810687571.0));
        assert_eq!(cookies[1].same_site, SameSite::None);
        assert_eq!(cookies[1].expires, None);
        assert_eq!(cookies[1].path, "/");
        assert!(parse_cookie_json("not json").is_err());
    }
}
