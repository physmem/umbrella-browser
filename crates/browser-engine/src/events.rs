use std::cell::RefCell;
use std::collections::VecDeque;
use std::fmt;
use std::rc::Rc;

use std::path::PathBuf;

use browser_core::TabId;
use browser_core::vault::{Credential, Origin};

use crate::Shortcut;

#[derive(Debug)]
pub enum TabEvent {
    Ready,
    CreateFailed(String),
    NavigationStarting {
        url: String,
    },
    NavigationCompleted {
        success: bool,
    },
    UrlChanged {
        url: String,
    },
    TitleChanged {
        title: String,
    },
    AudioChanged {
        playing: bool,
    },
    NewWindowRequested {
        url: String,
    },
    /// A page tried to open a window without a click; it was not opened.
    PopupBlocked {
        url: String,
    },
    LoginFormSeen {
        origin: Origin,
    },
    LoginSubmitted {
        origin: Origin,
        credential: Credential,
    },
    Suspended {
        ok: bool,
    },
    ProcessFailed,
    /// The page took keyboard focus, e.g. the user clicked into it.
    PageFocused,
    /// The page opened a WebRTC connection.
    WebRtcUsed,
    /// A cookie import finished; `count` cookies were set in this profile.
    CookiesImported {
        count: usize,
    },
    /// A site asked for a permission; answer with `TabView::resolve_permission`.
    PermissionRequested {
        request: u64,
        origin: String,
        kind: browser_core::permissions::PermissionKind,
    },
    /// Find in page: `active` is 1-based, 0 while nothing is selected.
    FindResult {
        active: u32,
        count: u32,
    },
    Shortcut(Shortcut),
    /// A page element (usually a video) entered or left fullscreen.
    FullscreenChanged {
        on: bool,
    },
    /// PNG bytes of the page's icon; empty when the page has none.
    FaviconChanged {
        png: Vec<u8>,
    },
    #[cfg(windows)]
    DownloadStarted {
        download: crate::DownloadId,
        file: PathBuf,
        total: Option<u64>,
    },
    /// "Ask where to save" is on: answer with `TabView::resolve_download`.
    #[cfg(windows)]
    DownloadSaveAs {
        download: crate::DownloadId,
        suggested: PathBuf,
    },
    #[cfg(windows)]
    DownloadProgress {
        download: crate::DownloadId,
        received: u64,
        total: Option<u64>,
    },
    #[cfg(windows)]
    DownloadFinished {
        download: crate::DownloadId,
        ok: bool,
    },
}

#[derive(Debug)]
pub struct EngineEvent {
    pub tab: TabId,
    pub event: TabEvent,
}

type Waker = Rc<dyn Fn()>;

/// Single-threaded mailbox from WebView2 callbacks to the UI. Callbacks only
/// push here, so they never re-enter UI state mid-update.
#[derive(Clone, Default)]
pub struct EventQueue {
    events: Rc<RefCell<VecDeque<EngineEvent>>>,
    waker: Rc<RefCell<Option<Waker>>>,
}

impl EventQueue {
    pub fn new() -> Self {
        Self::default()
    }

    /// Called after every push, e.g. to schedule a UI refresh.
    pub fn set_waker(&self, waker: impl Fn() + 'static) {
        *self.waker.borrow_mut() = Some(Rc::new(waker));
    }

    pub fn push(&self, tab: TabId, event: TabEvent) {
        self.events
            .borrow_mut()
            .push_back(EngineEvent { tab, event });
        // Clone out first so the waker may drain or push without a borrow held.
        let waker = self.waker.borrow().clone();
        if let Some(wake) = waker {
            wake();
        }
    }

    pub fn drain(&self) -> Vec<EngineEvent> {
        self.events.borrow_mut().drain(..).collect()
    }

    pub fn is_empty(&self) -> bool {
        self.events.borrow().is_empty()
    }
}

impl fmt::Debug for EventQueue {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        write!(f, "EventQueue({} pending)", self.events.borrow().len())
    }
}

#[cfg(test)]
mod tests {
    use std::cell::Cell;

    use super::*;

    #[test]
    fn waker_can_drain_reentrantly() {
        let queue = EventQueue::new();
        let seen = Rc::new(Cell::new(0));
        let (q, s) = (queue.clone(), seen.clone());
        queue.set_waker(move || s.set(s.get() + q.drain().len()));
        queue.push(TabId(1), TabEvent::Ready);
        queue.push(TabId(2), TabEvent::ProcessFailed);
        assert_eq!(seen.get(), 2);
        assert!(queue.is_empty());
    }
}
