//! What the chrome can learn about a page without devtools: the main
//! document's HTTP response, a few in-page measurements, and how much memory
//! each renderer process uses.

use std::cell::RefCell;
use std::rc::Rc;

use webview2_com::Microsoft::Web::WebView2::Win32::*;
use webview2_com::{
    GetProcessExtendedInfosCompletedHandler, WebResourceResponseReceivedEventHandler, take_pwstr,
};
use windows::Win32::Foundation::CloseHandle;
use windows::Win32::System::ProcessStatus::{
    GetProcessMemoryInfo, PROCESS_MEMORY_COUNTERS, PROCESS_MEMORY_COUNTERS_EX2,
};
use windows::Win32::System::Threading::{OpenProcess, PROCESS_QUERY_LIMITED_INFORMATION};
use windows::core::{BOOL, Interface, PWSTR};

use crate::{EngineError, Profile};

/// Pages rarely send more; the cap keeps a hostile response small.
const MAX_HEADERS: usize = 128;

/// The HTTP response that delivered the current page.
#[derive(Debug, Clone, Default, PartialEq, Eq)]
pub struct DocumentResponse {
    pub url: String,
    pub status: u16,
    pub reason: String,
    pub headers: Vec<(String, String)>,
}

impl DocumentResponse {
    /// First value of `name`, compared case-insensitively.
    pub fn header(&self, name: &str) -> Option<&str> {
        self.headers
            .iter()
            .find(|(n, _)| n.eq_ignore_ascii_case(name))
            .map(|(_, v)| v.as_str())
    }
}

/// Measured inside the page; every field is optional on old pages.
pub const PAGE_STATS_SCRIPT: &str = r#"(() => {
  const nav = performance.getEntriesByType('navigation')[0] || {};
  return {
    protocol: nav.nextHopProtocol || '',
    transfer_bytes: Math.round(nav.transferSize || 0),
    body_bytes: Math.round(nav.decodedBodySize || 0),
    load_ms: Math.round(Math.max(0, (nav.loadEventEnd || nav.domContentLoadedEventEnd || 0) - (nav.startTime || 0))),
    resources: performance.getEntriesByType('resource').length,
    script_cookies: document.cookie ? document.cookie.split(';').length : 0,
    scripts: document.scripts.length,
    iframes: document.querySelectorAll('iframe').length,
    secure_context: window.isSecureContext
  };
})()"#;

/// Remembers the response for the URL the main frame is navigating to.
pub(crate) fn watch_document(
    webview: &ICoreWebView2,
    nav_target: Rc<RefCell<String>>,
    slot: Rc<RefCell<Option<DocumentResponse>>>,
) -> Result<(), EngineError> {
    let webview2: ICoreWebView2_2 = webview.cast()?;
    let handler = WebResourceResponseReceivedEventHandler::create(Box::new(move |_, args| {
        let Some(args) = args else {
            return Ok(());
        };
        let request = unsafe { args.Request()? };
        let url = read_pwstr(|p| unsafe { request.Uri(p) })?;
        // Every image and script passes through here; only the page counts.
        if url != *nav_target.borrow() {
            return Ok(());
        }
        let response = unsafe { args.Response()? };
        let mut status = 0;
        unsafe { response.StatusCode(&mut status)? };
        let reason = read_pwstr(|p| unsafe { response.ReasonPhrase(p) })?;
        let headers = read_headers(&unsafe { response.Headers()? })?;
        *slot.borrow_mut() = Some(DocumentResponse {
            url,
            status: u16::try_from(status).unwrap_or(0),
            reason,
            headers,
        });
        Ok(())
    }));
    let mut token = 0;
    unsafe { webview2.add_WebResourceResponseReceived(&handler, &mut token)? };
    Ok(())
}

fn read_headers(
    headers: &ICoreWebView2HttpResponseHeaders,
) -> windows::core::Result<Vec<(String, String)>> {
    let iter = unsafe { headers.GetIterator()? };
    let mut out = Vec::new();
    let mut has = BOOL::default();
    unsafe { iter.HasCurrentHeader(&mut has)? };
    while has.as_bool() && out.len() < MAX_HEADERS {
        let (mut name, mut value) = (PWSTR::null(), PWSTR::null());
        unsafe { iter.GetCurrentHeader(&mut name, &mut value)? };
        out.push((take_pwstr(name), take_pwstr(value)));
        unsafe { iter.MoveNext(&mut has)? };
    }
    out.sort_by_key(|(n, _)| n.to_ascii_lowercase());
    Ok(out)
}

/// One renderer process and the main frames (tabs) it draws.
#[derive(Debug, Clone, Default)]
pub struct RendererMemory {
    pub pid: u32,
    /// Private working set, the "Memory" column in Task Manager.
    pub bytes: u64,
    pub main_frames: Vec<u32>,
}

impl Profile {
    /// Lists renderer processes with their memory. Needs runtime 1.0.2210+;
    /// older runtimes report an error and the chrome shows nothing.
    pub fn renderer_memory(
        &self,
        on_done: impl FnOnce(Vec<RendererMemory>) + 'static,
    ) -> Result<(), EngineError> {
        let env13: ICoreWebView2Environment13 = self.env.cast()?;
        let handler =
            GetProcessExtendedInfosCompletedHandler::create(Box::new(move |result, infos| {
                let list = match (result, infos) {
                    (Ok(()), Some(infos)) => renderers(&infos).unwrap_or_default(),
                    _ => Vec::new(),
                };
                on_done(list);
                Ok(())
            }));
        unsafe { env13.GetProcessExtendedInfos(&handler)? };
        Ok(())
    }
}

fn renderers(
    infos: &ICoreWebView2ProcessExtendedInfoCollection,
) -> windows::core::Result<Vec<RendererMemory>> {
    let mut count = 0;
    unsafe { infos.Count(&mut count)? };
    let mut out = Vec::new();
    for i in 0..count {
        let info = unsafe { infos.GetValueAtIndex(i)? };
        let process = unsafe { info.ProcessInfo()? };
        let mut kind = COREWEBVIEW2_PROCESS_KIND::default();
        unsafe { process.Kind(&mut kind)? };
        if kind != COREWEBVIEW2_PROCESS_KIND_RENDERER {
            continue;
        }
        let mut pid = 0;
        unsafe { process.ProcessId(&mut pid)? };
        let Ok(pid) = u32::try_from(pid) else {
            continue;
        };
        out.push(RendererMemory {
            pid,
            bytes: private_working_set(pid).unwrap_or(0),
            main_frames: main_frames(&unsafe { info.AssociatedFrameInfos()? })?,
        });
    }
    Ok(out)
}

fn main_frames(frames: &ICoreWebView2FrameInfoCollection) -> windows::core::Result<Vec<u32>> {
    let iter = unsafe { frames.GetIterator()? };
    let mut out = Vec::new();
    let mut has = BOOL::default();
    unsafe { iter.HasCurrent(&mut has)? };
    while has.as_bool() {
        let frame = unsafe { iter.GetCurrent()? };
        if let Ok(frame2) = frame.cast::<ICoreWebView2FrameInfo2>() {
            let mut kind = COREWEBVIEW2_FRAME_KIND::default();
            let mut id = 0;
            unsafe {
                frame2.FrameKind(&mut kind)?;
                frame2.FrameId(&mut id)?;
            }
            if kind == COREWEBVIEW2_FRAME_KIND_MAIN_FRAME {
                out.push(id);
            }
        }
        unsafe { iter.MoveNext(&mut has)? };
    }
    Ok(out)
}

fn private_working_set(pid: u32) -> Option<u64> {
    let process = unsafe { OpenProcess(PROCESS_QUERY_LIMITED_INFORMATION, false, pid) }.ok()?;
    let mut counters = PROCESS_MEMORY_COUNTERS_EX2 {
        cb: std::mem::size_of::<PROCESS_MEMORY_COUNTERS_EX2>() as u32,
        ..Default::default()
    };
    let read = unsafe {
        GetProcessMemoryInfo(
            process,
            (&mut counters as *mut PROCESS_MEMORY_COUNTERS_EX2).cast::<PROCESS_MEMORY_COUNTERS>(),
            counters.cb,
        )
    };
    let _ = unsafe { CloseHandle(process) };
    read.ok()?;
    Some(counters.PrivateWorkingSetSize as u64)
}

fn read_pwstr(
    get: impl FnOnce(*mut PWSTR) -> windows::core::Result<()>,
) -> windows::core::Result<String> {
    let mut raw = PWSTR::null();
    get(&mut raw)?;
    Ok(take_pwstr(raw))
}
