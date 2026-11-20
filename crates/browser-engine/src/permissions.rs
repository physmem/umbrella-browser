//! Permission requests (camera, location, ...). Remembered answers apply
//! at once; anything else waits on a deferral until the chrome asks the user.

use std::cell::RefCell;
use std::collections::HashMap;
use std::rc::Rc;
use std::sync::atomic::{AtomicU64, Ordering};

use browser_core::TabId;
use browser_core::permissions::PermissionKind;
use webview2_com::Microsoft::Web::WebView2::Win32::*;
use webview2_com::{PermissionRequestedEventHandler, take_pwstr};
use windows::core::{Interface, PWSTR};

use crate::{EngineError, EventQueue, TabEvent};

/// `(origin, kind) -> Some(allow)` for remembered decisions.
pub type PermissionPolicy = Rc<dyn Fn(&str, PermissionKind) -> Option<bool>>;

pub(crate) type Pending = Rc<
    RefCell<
        HashMap<
            u64,
            (
                ICoreWebView2PermissionRequestedEventArgs,
                ICoreWebView2Deferral,
            ),
        >,
    >,
>;

static NEXT_REQUEST: AtomicU64 = AtomicU64::new(1);

pub(crate) fn watch(
    webview: &ICoreWebView2,
    id: TabId,
    queue: EventQueue,
    policy: PermissionPolicy,
    pending: Pending,
) -> Result<(), EngineError> {
    let handler = PermissionRequestedEventHandler::create(Box::new(move |_, args| {
        let Some(args) = args else {
            return Ok(());
        };
        let mut raw_kind = COREWEBVIEW2_PERMISSION_KIND::default();
        unsafe { args.PermissionKind(&mut raw_kind)? };
        // Autoplay and unknown kinds keep WebView2's own behaviour.
        let Some(kind) = kind_of(raw_kind) else {
            return Ok(());
        };
        let mut raw = PWSTR::null();
        unsafe { args.Uri(&mut raw)? };
        let Some(origin) = origin_of(&take_pwstr(raw)) else {
            return Ok(());
        };
        // We remember decisions ourselves, per exact origin.
        if let Ok(args3) = args.cast::<ICoreWebView2PermissionRequestedEventArgs3>() {
            let _ = unsafe { args3.SetSavesInProfile(false) };
        }
        if let Some(allow) = policy(&origin, kind) {
            unsafe { args.SetState(state(Some(allow)))? };
            return Ok(());
        }
        let deferral = unsafe { args.GetDeferral()? };
        let request = NEXT_REQUEST.fetch_add(1, Ordering::Relaxed);
        pending.borrow_mut().insert(request, (args, deferral));
        queue.push(
            id,
            TabEvent::PermissionRequested {
                request,
                origin,
                kind,
            },
        );
        Ok(())
    }));
    let mut token = 0;
    unsafe { webview.add_PermissionRequested(&handler, &mut token)? };
    Ok(())
}

/// `Some(allow)` answers; `None` refuses this time without remembering.
pub(crate) fn resolve(
    pending: &Pending,
    request: u64,
    allow: Option<bool>,
) -> Result<(), EngineError> {
    let Some((args, deferral)) = pending.borrow_mut().remove(&request) else {
        return Ok(());
    };
    let set = unsafe { args.SetState(state(allow.or(Some(false)))) };
    // Complete even if setting the state failed, or the page hangs waiting.
    unsafe { deferral.Complete()? };
    set?;
    Ok(())
}

fn state(allow: Option<bool>) -> COREWEBVIEW2_PERMISSION_STATE {
    match allow {
        Some(true) => COREWEBVIEW2_PERMISSION_STATE_ALLOW,
        Some(false) => COREWEBVIEW2_PERMISSION_STATE_DENY,
        None => COREWEBVIEW2_PERMISSION_STATE_DEFAULT,
    }
}

fn kind_of(kind: COREWEBVIEW2_PERMISSION_KIND) -> Option<PermissionKind> {
    Some(match kind {
        COREWEBVIEW2_PERMISSION_KIND_CAMERA => PermissionKind::Camera,
        COREWEBVIEW2_PERMISSION_KIND_MICROPHONE => PermissionKind::Microphone,
        COREWEBVIEW2_PERMISSION_KIND_GEOLOCATION => PermissionKind::Location,
        COREWEBVIEW2_PERMISSION_KIND_NOTIFICATIONS => PermissionKind::Notifications,
        COREWEBVIEW2_PERMISSION_KIND_CLIPBOARD_READ => PermissionKind::ClipboardRead,
        COREWEBVIEW2_PERMISSION_KIND_OTHER_SENSORS => PermissionKind::Sensors,
        COREWEBVIEW2_PERMISSION_KIND_MIDI_SYSTEM_EXCLUSIVE_MESSAGES => PermissionKind::Midi,
        COREWEBVIEW2_PERMISSION_KIND_WINDOW_MANAGEMENT => PermissionKind::WindowManagement,
        COREWEBVIEW2_PERMISSION_KIND_LOCAL_FONTS => PermissionKind::LocalFonts,
        COREWEBVIEW2_PERMISSION_KIND_MULTIPLE_AUTOMATIC_DOWNLOADS => {
            PermissionKind::MultipleDownloads
        }
        COREWEBVIEW2_PERMISSION_KIND_FILE_READ_WRITE => PermissionKind::FileEditing,
        _ => return None,
    })
}

/// `https://meet.example.com:8443/room` -> `https://meet.example.com:8443`.
pub fn origin_of(url: &str) -> Option<String> {
    let parsed = url::Url::parse(url).ok()?;
    let origin = parsed.origin();
    origin.is_tuple().then(|| origin.ascii_serialization())
}

#[cfg(test)]
mod tests {
    #[test]
    fn origins_drop_path_and_keep_port() {
        assert_eq!(
            super::origin_of("https://meet.example.com:8443/room?x=1").as_deref(),
            Some("https://meet.example.com:8443")
        );
        assert_eq!(super::origin_of("about:blank"), None);
    }
}
