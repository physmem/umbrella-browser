//! Browser extensions (unpacked folders) through WebView2's profile API.
//! Needs runtime 1.0.2420+ and a profile started with extensions enabled.

use std::path::Path;

use webview2_com::Microsoft::Web::WebView2::Win32::*;
use webview2_com::{
    BrowserExtensionEnableCompletedHandler, BrowserExtensionRemoveCompletedHandler,
    ProfileAddBrowserExtensionCompletedHandler, ProfileGetBrowserExtensionsCompletedHandler,
    take_pwstr,
};
use windows::core::{BOOL, HSTRING, Interface, PWSTR};

use crate::EngineError;

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct ExtensionInfo {
    pub id: String,
    pub name: String,
    pub enabled: bool,
}

pub(crate) fn profile7(webview: &ICoreWebView2) -> Result<ICoreWebView2Profile7, EngineError> {
    let webview13: ICoreWebView2_13 = webview.cast()?;
    Ok(unsafe { webview13.Profile()? }.cast()?)
}

pub(crate) fn install(
    webview: &ICoreWebView2,
    folder: &Path,
    on_done: impl FnOnce(Result<ExtensionInfo, String>) + 'static,
) -> Result<(), EngineError> {
    let profile = profile7(webview)?;
    let handler =
        ProfileAddBrowserExtensionCompletedHandler::create(Box::new(move |result, ext| {
            let info = match (result, ext) {
                (Ok(()), Some(ext)) => info_of(&ext).map_err(|e| e.to_string()),
                (Err(e), _) => Err(e.message().to_string()),
                (Ok(()), None) => Err("WebView2 returned no extension".to_owned()),
            };
            on_done(info);
            Ok(())
        }));
    unsafe { profile.AddBrowserExtension(&HSTRING::from(folder.as_os_str()), &handler)? };
    Ok(())
}

pub(crate) fn list(
    webview: &ICoreWebView2,
    on_done: impl FnOnce(Vec<ExtensionInfo>) + 'static,
) -> Result<(), EngineError> {
    let profile = profile7(webview)?;
    let handler =
        ProfileGetBrowserExtensionsCompletedHandler::create(Box::new(move |result, list| {
            let infos = match (result, list) {
                (Ok(()), Some(list)) => infos_of(&list).unwrap_or_default(),
                _ => Vec::new(),
            };
            on_done(infos);
            Ok(())
        }));
    unsafe { profile.GetBrowserExtensions(&handler)? };
    Ok(())
}

/// Runs `act` on the extension with `id`, if the profile has it.
pub(crate) fn with_extension(
    webview: &ICoreWebView2,
    id: String,
    act: impl FnOnce(ICoreWebView2BrowserExtension) + 'static,
) -> Result<(), EngineError> {
    let profile = profile7(webview)?;
    let handler =
        ProfileGetBrowserExtensionsCompletedHandler::create(Box::new(move |result, list| {
            let (Ok(()), Some(list)) = (result, list) else {
                return Ok(());
            };
            let mut count = 0;
            unsafe { list.Count(&mut count)? };
            for i in 0..count {
                let ext = unsafe { list.GetValueAtIndex(i)? };
                if info_of(&ext).is_ok_and(|info| info.id == id) {
                    act(ext);
                    break;
                }
            }
            Ok(())
        }));
    unsafe { profile.GetBrowserExtensions(&handler)? };
    Ok(())
}

pub(crate) fn set_enabled(
    webview: &ICoreWebView2,
    id: String,
    enabled: bool,
) -> Result<(), EngineError> {
    with_extension(webview, id, move |ext| {
        let done = BrowserExtensionEnableCompletedHandler::create(Box::new(|_| Ok(())));
        let _ = unsafe { ext.Enable(enabled, &done) };
    })
}

pub(crate) fn remove(webview: &ICoreWebView2, id: String) -> Result<(), EngineError> {
    with_extension(webview, id, |ext| {
        let done = BrowserExtensionRemoveCompletedHandler::create(Box::new(|_| Ok(())));
        let _ = unsafe { ext.Remove(&done) };
    })
}

fn infos_of(list: &ICoreWebView2BrowserExtensionList) -> windows::core::Result<Vec<ExtensionInfo>> {
    let mut count = 0;
    unsafe { list.Count(&mut count)? };
    (0..count)
        .map(|i| info_of(&unsafe { list.GetValueAtIndex(i)? }))
        .collect()
}

fn info_of(ext: &ICoreWebView2BrowserExtension) -> windows::core::Result<ExtensionInfo> {
    let (mut id, mut name) = (PWSTR::null(), PWSTR::null());
    let mut enabled = BOOL::default();
    unsafe {
        ext.Id(&mut id)?;
        ext.Name(&mut name)?;
        ext.IsEnabled(&mut enabled)?;
    }
    Ok(ExtensionInfo {
        id: take_pwstr(id),
        name: take_pwstr(name),
        enabled: enabled.as_bool(),
    })
}
