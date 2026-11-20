//! Writing cookies into the WebView2 profile, e.g. importing a signed-in
//! session exported from another browser. A site's existing cookies are
//! cleared first, because a logged-out visit leaves cookies that the site
//! binds the new session against and the import would otherwise be rejected.

use std::cell::{Cell, RefCell};
use std::collections::BTreeSet;
use std::rc::Rc;

use webview2_com::Microsoft::Web::WebView2::Win32::*;
use webview2_com::{GetCookiesCompletedHandler, take_pwstr};
use windows::core::{HSTRING, Interface, PWSTR};

use crate::{EngineError, TabEvent};

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum SameSite {
    None,
    Lax,
    Strict,
}

/// One cookie to add. `expires` is seconds since the Unix epoch; `None`
/// makes it a session cookie that goes away when the profile clears.
#[derive(Debug, Clone)]
pub struct ImportCookie {
    pub name: String,
    pub value: String,
    pub domain: String,
    pub path: String,
    pub secure: bool,
    pub http_only: bool,
    pub same_site: SameSite,
    pub expires: Option<f64>,
}

impl crate::TabView {
    /// Clears each cookie site's existing cookies, adds the imported ones,
    /// then reports `TabEvent::CookiesImported` so the page can reload signed
    /// in. Everything runs on the UI thread through WebView2's own callbacks.
    pub fn import_cookies(&self, cookies: Vec<ImportCookie>) -> Result<(), EngineError> {
        let webview2: ICoreWebView2_2 = self.webview().cast()?;
        let manager = unsafe { webview2.CookieManager()? };
        let (id, queue) = (self.id(), self.queue_handle());

        let hosts: BTreeSet<String> = cookies
            .iter()
            .map(|c| c.domain.trim_start_matches('.').to_owned())
            .filter(|h| !h.is_empty())
            .collect();
        if hosts.is_empty() {
            let added = add_all(&manager, &cookies);
            queue.push(id, TabEvent::CookiesImported { count: added });
            return Ok(());
        }

        // Add the imported cookies only once every host has been cleared.
        let remaining = Rc::new(Cell::new(hosts.len()));
        let pending = Rc::new(RefCell::new(Some((manager.clone(), cookies, queue, id))));
        for host in hosts {
            let uri = HSTRING::from(format!("https://{host}/"));
            let (mgr, remaining, pending) = (manager.clone(), remaining.clone(), pending.clone());
            let handler = GetCookiesCompletedHandler::create(Box::new(move |result, list| {
                if let (Ok(()), Some(list)) = (result, list) {
                    clear_all(&mgr, &list);
                }
                remaining.set(remaining.get().saturating_sub(1));
                if remaining.get() == 0
                    && let Some((mgr, cookies, queue, id)) = pending.borrow_mut().take()
                {
                    let added = add_all(&mgr, &cookies);
                    queue.push(id, TabEvent::CookiesImported { count: added });
                }
                Ok(())
            }));
            unsafe { manager.GetCookies(&uri, &handler)? };
        }
        Ok(())
    }
}

fn clear_all(manager: &ICoreWebView2CookieManager, list: &ICoreWebView2CookieList) {
    let mut count = 0;
    if unsafe { list.Count(&mut count) }.is_err() {
        return;
    }
    for i in 0..count {
        let Ok(cookie) = (unsafe { list.GetValueAtIndex(i) }) else {
            continue;
        };
        let (Ok(name), Ok(domain), Ok(path)) = (
            read_pwstr(|p| unsafe { cookie.Name(p) }),
            read_pwstr(|p| unsafe { cookie.Domain(p) }),
            read_pwstr(|p| unsafe { cookie.Path(p) }),
        ) else {
            continue;
        };
        // Overwrite the cookie with one that expired in the past: the key is
        // (name, domain, path), so matching all three (domain kept as stored,
        // with its leading dot) drops it.
        let dead = unsafe {
            manager.CreateCookie(
                &HSTRING::from(&name),
                &HSTRING::from(""),
                &HSTRING::from(domain),
                &HSTRING::from(&path),
            )
        };
        if let Ok(dead) = dead {
            let _ = unsafe { dead.SetExpires(1.0) };
            let _ = unsafe { manager.AddOrUpdateCookie(&dead) };
        }
    }
}

fn add_all(manager: &ICoreWebView2CookieManager, cookies: &[ImportCookie]) -> usize {
    cookies
        .iter()
        .filter(|c| add_one(manager, c).is_ok())
        .count()
}

fn add_one(manager: &ICoreWebView2CookieManager, c: &ImportCookie) -> windows::core::Result<()> {
    // Keep the domain exactly as exported: a leading dot (".example.com")
    // makes a domain cookie sent to every subdomain, which is what sites set
    // their session cookies as. Stripping it makes a host-only cookie that
    // does not match, so the real session is never recognised.
    let cookie = unsafe {
        manager.CreateCookie(
            &HSTRING::from(&c.name),
            &HSTRING::from(&c.value),
            &HSTRING::from(&c.domain),
            &HSTRING::from(&c.path),
        )?
    };
    unsafe {
        cookie.SetIsSecure(c.secure)?;
        cookie.SetIsHttpOnly(c.http_only)?;
        cookie.SetSameSite(same_site(c.same_site))?;
        if let Some(expires) = c.expires {
            cookie.SetExpires(expires)?;
        }
        manager.AddOrUpdateCookie(&cookie)?;
    }
    Ok(())
}

fn same_site(kind: SameSite) -> COREWEBVIEW2_COOKIE_SAME_SITE_KIND {
    match kind {
        SameSite::None => COREWEBVIEW2_COOKIE_SAME_SITE_KIND_NONE,
        SameSite::Lax => COREWEBVIEW2_COOKIE_SAME_SITE_KIND_LAX,
        SameSite::Strict => COREWEBVIEW2_COOKIE_SAME_SITE_KIND_STRICT,
    }
}

fn read_pwstr(
    get: impl FnOnce(*mut PWSTR) -> windows::core::Result<()>,
) -> windows::core::Result<String> {
    let mut raw = PWSTR::null();
    get(&mut raw)?;
    Ok(take_pwstr(raw))
}
