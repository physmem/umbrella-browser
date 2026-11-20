//! The few Win32 calls the chrome needs directly.

use browser_core::theme::Rgba;
use gpui::Window;
use raw_window_handle::{HasWindowHandle, RawWindowHandle};
use windows::Win32::Foundation::HWND;
use windows::Win32::Graphics::Gdi::{GetSysColor, SYS_COLOR_INDEX};
use windows::Win32::UI::Accessibility::{HCF_HIGHCONTRASTON, HIGHCONTRASTW};
use windows::Win32::UI::Input::KeyboardAndMouse::SetFocus;
use windows::Win32::UI::WindowsAndMessaging::{
    IsIconic, SPI_GETCLIENTAREAANIMATION, SPI_GETHIGHCONTRAST, SW_RESTORE,
    SYSTEM_PARAMETERS_INFO_UPDATE_FLAGS, SetForegroundWindow, ShowWindow, SystemParametersInfoW,
};
use windows::core::BOOL;

pub fn hwnd(window: &Window) -> Option<HWND> {
    // The trait method, not GPUI's inherent `window_handle`, which returns
    // GPUI's own handle type.
    match HasWindowHandle::window_handle(window).ok()?.as_raw() {
        RawWindowHandle::Win32(handle) => Some(HWND(handle.hwnd.get() as *mut _)),
        _ => None,
    }
}

/// Windows Settings > Accessibility > Visual effects > Animation effects.
pub fn reduced_motion() -> bool {
    let mut enabled = BOOL(1);
    // SAFETY: SPI_GETCLIENTAREAANIMATION writes one BOOL to the pointer.
    let ok = unsafe {
        SystemParametersInfoW(
            SPI_GETCLIENTAREAANIMATION,
            0,
            Some((&mut enabled as *mut BOOL).cast()),
            SYSTEM_PARAMETERS_INFO_UPDATE_FLAGS(0),
        )
    };
    ok.is_ok() && !enabled.as_bool()
}

pub fn high_contrast() -> bool {
    let mut info = HIGHCONTRASTW {
        cbSize: std::mem::size_of::<HIGHCONTRASTW>() as u32,
        ..Default::default()
    };
    // SAFETY: SPI_GETHIGHCONTRAST fills a HIGHCONTRASTW whose cbSize is set.
    let ok = unsafe {
        SystemParametersInfoW(
            SPI_GETHIGHCONTRAST,
            info.cbSize,
            Some((&mut info as *mut HIGHCONTRASTW).cast()),
            SYSTEM_PARAMETERS_INFO_UPDATE_FLAGS(0),
        )
    };
    ok.is_ok() && info.dwFlags.contains(HCF_HIGHCONTRASTON)
}

/// A click on the chrome does not pull keyboard focus away from a focused
/// WebView2 child window on its own, so take it back explicitly.
pub fn take_focus(hwnd: HWND) {
    // SAFETY: `hwnd` is our live top-level window on this thread.
    let _ = unsafe { SetFocus(Some(hwnd)) };
}

/// A system color (`COLOR_*` index); COLORREF is 0x00BBGGRR.
pub fn sys_color(index: i32) -> Rgba {
    // SAFETY: GetSysColor has no preconditions.
    let bgr = unsafe { GetSysColor(SYS_COLOR_INDEX(index)) };
    let [r, g, b, _] = bgr.to_le_bytes();
    Rgba { r, g, b, a: 0xff }
}

/// Restores and raises the window. Works without tricks only when the
/// caller holds foreground rights, which a forwarding launch grants us.
pub fn bring_to_front(hwnd: HWND) {
    // SAFETY: plain calls on our own top-level window.
    unsafe {
        if IsIconic(hwnd).as_bool() {
            let _ = ShowWindow(hwnd, SW_RESTORE);
        }
        let _ = SetForegroundWindow(hwnd);
    }
}
