//! Find in page through WebView2's Find API, with its own dialog hidden so
//! the chrome's find bar drives it. Needs runtime 1.0.2849+.

use webview2_com::Microsoft::Web::WebView2::Win32::*;
use webview2_com::{
    FindActiveMatchIndexChangedEventHandler, FindMatchCountChangedEventHandler,
    FindStartCompletedHandler,
};
use windows::core::{HSTRING, Interface};

use browser_core::TabId;

use crate::{EngineError, EventQueue, TabEvent};

pub(crate) fn find_of(webview: &ICoreWebView2) -> Option<ICoreWebView2Find> {
    let webview28 = webview.cast::<ICoreWebView2_28>().ok()?;
    unsafe { webview28.Find() }.ok()
}

/// Reports match counts as `TabEvent::FindResult`.
pub(crate) fn watch(
    webview: &ICoreWebView2,
    id: TabId,
    queue: EventQueue,
) -> Result<(), EngineError> {
    let Some(find) = find_of(webview) else {
        return Ok(());
    };
    let report = move |queue: &EventQueue, find: Option<ICoreWebView2Find>| {
        let Some(find) = find else {
            return;
        };
        let (mut active, mut count) = (0, 0);
        unsafe {
            let _ = find.ActiveMatchIndex(&mut active);
            let _ = find.MatchCount(&mut count);
        }
        queue.push(
            id,
            TabEvent::FindResult {
                active: u32::try_from(active).unwrap_or(0),
                count: u32::try_from(count).unwrap_or(0),
            },
        );
    };
    let q = queue.clone();
    let on_count = FindMatchCountChangedEventHandler::create(Box::new(move |sender, _| {
        report(&q, sender);
        Ok(())
    }));
    let on_active = FindActiveMatchIndexChangedEventHandler::create(Box::new(move |sender, _| {
        report(&queue, sender);
        Ok(())
    }));
    let mut token = 0;
    unsafe {
        find.add_MatchCountChanged(&on_count, &mut token)?;
        find.add_ActiveMatchIndexChanged(&on_active, &mut token)?;
    }
    Ok(())
}

pub(crate) fn start(
    env: &ICoreWebView2Environment,
    webview: &ICoreWebView2,
    term: &str,
    case_sensitive: bool,
) -> Result<(), EngineError> {
    let find = find_of(webview).ok_or(EngineError::Missing)?;
    let env15: ICoreWebView2Environment15 = env.cast()?;
    let options = unsafe { env15.CreateFindOptions()? };
    unsafe {
        options.SetFindTerm(&HSTRING::from(term))?;
        options.SetIsCaseSensitive(case_sensitive)?;
        options.SetShouldHighlightAllMatches(true)?;
        options.SetSuppressDefaultFindDialog(true)?;
    }
    let done = FindStartCompletedHandler::create(Box::new(|_| Ok(())));
    unsafe { find.Start(&options, &done)? };
    Ok(())
}

pub(crate) fn step(webview: &ICoreWebView2, forward: bool) -> Result<(), EngineError> {
    let find = find_of(webview).ok_or(EngineError::Missing)?;
    unsafe {
        if forward {
            find.FindNext()?;
        } else {
            find.FindPrevious()?;
        }
    }
    Ok(())
}

pub(crate) fn stop(webview: &ICoreWebView2) -> Result<(), EngineError> {
    let find = find_of(webview).ok_or(EngineError::Missing)?;
    unsafe { find.Stop()? };
    Ok(())
}
