//! Downloads: WebView2's own download bubble is replaced by events the
//! chrome can show. Files land in the user's chosen folder (or WebView2's
//! default, the Downloads folder) with WebView2's own safety checks.

use std::cell::RefCell;
use std::collections::HashMap;
use std::path::{Path, PathBuf};
use std::rc::Rc;
use std::sync::atomic::{AtomicU64, Ordering};

use browser_core::TabId;
use webview2_com::Microsoft::Web::WebView2::Win32::*;
use webview2_com::{
    BytesReceivedChangedEventHandler, DownloadStartingEventHandler, StateChangedEventHandler,
    take_pwstr,
};
use windows::core::{HSTRING, Interface, PWSTR};

use crate::{EngineError, EventQueue, TabEvent};

#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub struct DownloadId(pub u64);

static NEXT_ID: AtomicU64 = AtomicU64::new(1);

/// Where downloads go. Shared with the event handler so changes in
/// settings apply to the next download without recreating the tab.
#[derive(Debug, Clone, Default, PartialEq, Eq)]
pub struct DownloadTarget {
    /// `None` keeps WebView2's default folder.
    pub folder: Option<PathBuf>,
    pub ask_where: bool,
}

/// Live operations, kept so a download can be cancelled from the chrome.
pub(crate) type Operations = Rc<RefCell<HashMap<DownloadId, ICoreWebView2DownloadOperation>>>;

/// Downloads held by a deferral while the user picks where to save them.
pub(crate) type PendingSaves = Rc<RefCell<HashMap<DownloadId, PendingSave>>>;

pub(crate) struct PendingSave {
    args: ICoreWebView2DownloadStartingEventArgs,
    deferral: ICoreWebView2Deferral,
}

#[derive(Clone)]
pub(crate) struct Watch {
    pub tab: TabId,
    pub queue: EventQueue,
    pub operations: Operations,
    pub pending: PendingSaves,
    pub target: Rc<RefCell<DownloadTarget>>,
}

pub(crate) fn watch(webview: &ICoreWebView2, w: Watch) -> Result<(), EngineError> {
    let webview4: ICoreWebView2_4 = webview.cast()?;
    let handler = DownloadStartingEventHandler::create(Box::new(move |_, args| {
        let Some(args) = args else {
            return Ok(());
        };
        // Handled hides WebView2's built-in download flyout; ours replaces it.
        unsafe { args.SetHandled(true)? };
        let id = DownloadId(NEXT_ID.fetch_add(1, Ordering::Relaxed));
        let suggested = read_path(|p| unsafe { args.ResultFilePath(p) })?;
        let target = w.target.borrow().clone();
        let file = match &target.folder {
            Some(folder) => unique_path(folder, &suggested),
            None => suggested,
        };
        if target.ask_where {
            // The deferral keeps the download waiting until `resolve` runs.
            let deferral = unsafe { args.GetDeferral()? };
            w.pending
                .borrow_mut()
                .insert(id, PendingSave { args, deferral });
            w.queue.push(
                w.tab,
                TabEvent::DownloadSaveAs {
                    download: id,
                    suggested: file,
                },
            );
            return Ok(());
        }
        begin(&args, id, &file, &w)
    }));
    let mut token = 0;
    unsafe { webview4.add_DownloadStarting(&handler, &mut token)? };
    Ok(())
}

/// Finishes a download that was waiting on a save dialog. `None` cancels it.
pub(crate) fn resolve(w: &Watch, id: DownloadId, path: Option<PathBuf>) -> Result<(), EngineError> {
    let Some(pending) = w.pending.borrow_mut().remove(&id) else {
        return Ok(());
    };
    let started = match &path {
        Some(file) => begin(&pending.args, id, file, w),
        None => unsafe { pending.args.SetCancel(true) },
    };
    // Complete even after an error, or the download hangs forever.
    unsafe { pending.deferral.Complete()? };
    started?;
    Ok(())
}

fn begin(
    args: &ICoreWebView2DownloadStartingEventArgs,
    id: DownloadId,
    file: &Path,
    w: &Watch,
) -> windows::core::Result<()> {
    unsafe { args.SetResultFilePath(&HSTRING::from(file.as_os_str()))? };
    let operation = unsafe { args.DownloadOperation()? };
    let total = total_bytes(&operation);
    track(&operation, id, w)?;
    w.operations.borrow_mut().insert(id, operation);
    w.queue.push(
        w.tab,
        TabEvent::DownloadStarted {
            download: id,
            file: file.to_path_buf(),
            total,
        },
    );
    Ok(())
}

fn track(
    operation: &ICoreWebView2DownloadOperation,
    id: DownloadId,
    w: &Watch,
) -> windows::core::Result<()> {
    let (tab, progress_queue) = (w.tab, w.queue.clone());
    let progress = BytesReceivedChangedEventHandler::create(Box::new(move |sender, _| {
        let Some(op) = sender else {
            return Ok(());
        };
        let mut received = 0i64;
        unsafe { op.BytesReceived(&mut received)? };
        let received = u64::try_from(received).unwrap_or(0);
        progress_queue.push(
            tab,
            TabEvent::DownloadProgress {
                download: id,
                received,
                total: total_bytes(&op),
            },
        );
        Ok(())
    }));
    let (queue, operations) = (w.queue.clone(), w.operations.clone());
    let state = StateChangedEventHandler::create(Box::new(move |sender, _| {
        let Some(op) = sender else {
            return Ok(());
        };
        let mut state = COREWEBVIEW2_DOWNLOAD_STATE::default();
        unsafe { op.State(&mut state)? };
        if state == COREWEBVIEW2_DOWNLOAD_STATE_IN_PROGRESS {
            return Ok(());
        }
        operations.borrow_mut().remove(&id);
        let ok = state == COREWEBVIEW2_DOWNLOAD_STATE_COMPLETED;
        queue.push(tab, TabEvent::DownloadFinished { download: id, ok });
        Ok(())
    }));
    let mut token = 0;
    unsafe {
        operation.add_BytesReceivedChanged(&progress, &mut token)?;
        operation.add_StateChanged(&state, &mut token)?;
    }
    Ok(())
}

pub(crate) fn cancel(operations: &Operations, id: DownloadId) -> Result<(), EngineError> {
    if let Some(op) = operations.borrow().get(&id) {
        unsafe { op.Cancel()? };
    }
    Ok(())
}

/// `folder/name.ext`, or `folder/name (2).ext` and so on when taken, the
/// same way WebView2 avoids overwriting in its default folder.
pub fn unique_path(folder: &Path, suggested: &Path) -> PathBuf {
    let name = suggested
        .file_name()
        .map(PathBuf::from)
        .unwrap_or_else(|| PathBuf::from("download"));
    let first = folder.join(&name);
    if !first.exists() {
        return first;
    }
    let stem = name
        .file_stem()
        .map(|s| s.to_string_lossy().into_owned())
        .unwrap_or_default();
    let ext = name
        .extension()
        .map(|e| format!(".{}", e.to_string_lossy()))
        .unwrap_or_default();
    (2..10_000)
        .map(|n| folder.join(format!("{stem} ({n}){ext}")))
        .find(|p| !p.exists())
        .unwrap_or(first)
}

/// `None` when the server did not send a length.
fn total_bytes(op: &ICoreWebView2DownloadOperation) -> Option<u64> {
    let mut total = 0i64;
    unsafe { op.TotalBytesToReceive(&mut total) }.ok()?;
    u64::try_from(total).ok().filter(|t| *t > 0)
}

fn read_path(
    get: impl FnOnce(*mut PWSTR) -> windows::core::Result<()>,
) -> windows::core::Result<PathBuf> {
    let mut raw = PWSTR::null();
    get(&mut raw)?;
    Ok(PathBuf::from(take_pwstr(raw)))
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn unique_path_skips_taken_names() -> std::io::Result<()> {
        let dir = std::env::temp_dir().join(format!("umbrella-dl-{}", std::process::id()));
        std::fs::create_dir_all(&dir)?;
        let suggested = Path::new(r"C:\Users\x\Downloads\report.pdf");
        assert_eq!(unique_path(&dir, suggested), dir.join("report.pdf"));
        std::fs::write(dir.join("report.pdf"), b"x")?;
        assert_eq!(unique_path(&dir, suggested), dir.join("report (2).pdf"));
        std::fs::remove_dir_all(&dir)?;
        Ok(())
    }
}
