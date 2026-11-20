//! Browser shortcuts. While a page has keyboard focus, WebView2 sees keys
//! first; the engine claims these and forwards them as `TabEvent::Shortcut`
//! so they behave the same whether the page or the chrome is focused.

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Shortcut {
    NewTab,
    CloseTab,
    ReopenTab,
    NextTab,
    PreviousTab,
    FocusOmnibox,
    CommandPalette,
    ZoomIn,
    ZoomOut,
    ZoomReset,
    ToggleBookmark,
    Back,
    Forward,
    History,
    Settings,
    MoveTabLeft,
    MoveTabRight,
    Find,
    /// Ctrl went up: ends an MRU Ctrl+Tab cycle.
    CtrlReleased,
}

#[derive(Debug, Clone, Copy, Default, PartialEq, Eq)]
pub struct Modifiers {
    pub ctrl: bool,
    pub shift: bool,
    pub alt: bool,
}

const VK_TAB: u32 = 0x09;
const VK_PRIOR: u32 = 0x21;
const VK_NEXT: u32 = 0x22;
const VK_CONTROL: u32 = 0x11;
const VK_LEFT: u32 = 0x25;
const VK_RIGHT: u32 = 0x27;
const VK_0: u32 = 0x30;
const VK_NUMPAD0: u32 = 0x60;
const VK_ADD: u32 = 0x6B;
const VK_SUBTRACT: u32 = 0x6D;
const VK_F2: u32 = 0x71;
const VK_OEM_PLUS: u32 = 0xBB;
const VK_OEM_COMMA: u32 = 0xBC;
const VK_OEM_MINUS: u32 = 0xBD;

/// Maps a key-down to a browser shortcut. Anything else stays with the page.
pub fn shortcut_for(vk: u32, m: Modifiers) -> Option<Shortcut> {
    if m.alt && !m.ctrl {
        return match vk {
            VK_LEFT => Some(Shortcut::Back),
            VK_RIGHT => Some(Shortcut::Forward),
            _ => None,
        };
    }
    if vk == VK_F2 && !m.ctrl {
        return Some(Shortcut::CommandPalette);
    }
    if !m.ctrl || m.alt {
        return None;
    }
    ctrl_shortcut(vk, m.shift)
}

fn ctrl_shortcut(vk: u32, shift: bool) -> Option<Shortcut> {
    let letter = char::from_u32(vk).filter(char::is_ascii_uppercase);
    match (letter, vk, shift) {
        (Some('T'), _, false) => Some(Shortcut::NewTab),
        (Some('T'), _, true) => Some(Shortcut::ReopenTab),
        (Some('W'), _, false) => Some(Shortcut::CloseTab),
        (Some('L'), _, false) => Some(Shortcut::FocusOmnibox),
        (Some('K'), _, false) => Some(Shortcut::CommandPalette),
        (Some('D'), _, false) => Some(Shortcut::ToggleBookmark),
        (Some('H'), _, false) => Some(Shortcut::History),
        (Some('F'), _, false) => Some(Shortcut::Find),
        (_, VK_OEM_COMMA, false) => Some(Shortcut::Settings),
        (_, VK_PRIOR, true) => Some(Shortcut::MoveTabLeft),
        (_, VK_NEXT, true) => Some(Shortcut::MoveTabRight),
        (_, VK_TAB, false) => Some(Shortcut::NextTab),
        (_, VK_TAB, true) => Some(Shortcut::PreviousTab),
        (_, VK_OEM_PLUS | VK_ADD, _) => Some(Shortcut::ZoomIn),
        (_, VK_OEM_MINUS | VK_SUBTRACT, _) => Some(Shortcut::ZoomOut),
        (_, VK_0 | VK_NUMPAD0, false) => Some(Shortcut::ZoomReset),
        _ => None,
    }
}

pub fn is_ctrl(vk: u32) -> bool {
    vk == VK_CONTROL
}

#[cfg(test)]
mod tests {
    use super::*;

    const CTRL: Modifiers = Modifiers {
        ctrl: true,
        shift: false,
        alt: false,
    };
    const CTRL_SHIFT: Modifiers = Modifiers {
        ctrl: true,
        shift: true,
        alt: false,
    };

    #[test]
    fn maps_browser_shortcuts() {
        assert_eq!(shortcut_for(u32::from(b'T'), CTRL), Some(Shortcut::NewTab));
        assert_eq!(
            shortcut_for(u32::from(b'T'), CTRL_SHIFT),
            Some(Shortcut::ReopenTab)
        );
        assert_eq!(
            shortcut_for(VK_TAB, CTRL_SHIFT),
            Some(Shortcut::PreviousTab)
        );
        assert_eq!(shortcut_for(VK_OEM_PLUS, CTRL), Some(Shortcut::ZoomIn));
        assert_eq!(
            shortcut_for(VK_F2, Modifiers::default()),
            Some(Shortcut::CommandPalette)
        );
        let alt = Modifiers {
            alt: true,
            ..Modifiers::default()
        };
        assert_eq!(shortcut_for(VK_LEFT, alt), Some(Shortcut::Back));
    }

    #[test]
    fn leaves_page_keys_alone() {
        assert_eq!(shortcut_for(u32::from(b'C'), CTRL), None);
        assert_eq!(shortcut_for(u32::from(b'T'), Modifiers::default()), None);
        let altgr = Modifiers {
            ctrl: true,
            shift: false,
            alt: true,
        };
        assert_eq!(shortcut_for(u32::from(b'T'), altgr), None);
    }
}
