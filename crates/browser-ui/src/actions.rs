//! Keyboard shortcuts while the chrome has focus. While a page has focus the
//! engine forwards the same shortcuts, so both paths end in `run_shortcut`.

use browser_engine::Shortcut;
use gpui::{App, Context, Div, KeyBinding, Stateful, actions, prelude::*};

use crate::browser::Browser;

actions!(
    umbrella,
    [
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
        MoveTabLeft,
        MoveTabRight,
        BrowseHistory,
        OpenSettings,
        FindInPage
    ]
);

pub fn bind(cx: &mut App) {
    let context = Some("Browser");
    cx.bind_keys([
        KeyBinding::new("ctrl-t", NewTab, context),
        KeyBinding::new("ctrl-w", CloseTab, context),
        KeyBinding::new("ctrl-shift-t", ReopenTab, context),
        KeyBinding::new("ctrl-tab", NextTab, context),
        KeyBinding::new("ctrl-shift-tab", PreviousTab, context),
        KeyBinding::new("ctrl-l", FocusOmnibox, context),
        KeyBinding::new("alt-d", FocusOmnibox, context),
        KeyBinding::new("ctrl-k", CommandPalette, context),
        KeyBinding::new("f2", CommandPalette, context),
        KeyBinding::new("ctrl-=", ZoomIn, context),
        KeyBinding::new("ctrl-+", ZoomIn, context),
        KeyBinding::new("ctrl--", ZoomOut, context),
        KeyBinding::new("ctrl-0", ZoomReset, context),
        KeyBinding::new("ctrl-d", ToggleBookmark, context),
        KeyBinding::new("alt-left", Back, context),
        KeyBinding::new("alt-right", Forward, context),
        KeyBinding::new("ctrl-shift-pageup", MoveTabLeft, context),
        KeyBinding::new("ctrl-h", BrowseHistory, context),
        KeyBinding::new("ctrl-,", OpenSettings, context),
        KeyBinding::new("ctrl-f", FindInPage, context),
        KeyBinding::new("ctrl-shift-pagedown", MoveTabRight, context),
    ]);
}

/// Attaches every shortcut listener to the root element. Actions bubble up
/// the focus path, so they must live on an ancestor of whatever has focus.
pub fn route(root: Stateful<Div>, cx: &mut Context<Browser>) -> Stateful<Div> {
    macro_rules! route {
        ($el:expr, $($action:ident => $shortcut:expr),* $(,)?) => {
            $el$(.on_action(cx.listener(|this, _: &$action, window, cx| {
                this.run_shortcut($shortcut, Some(window), cx)
            })))*
        };
    }
    route!(root,
        NewTab => Shortcut::NewTab,
        CloseTab => Shortcut::CloseTab,
        ReopenTab => Shortcut::ReopenTab,
        NextTab => Shortcut::NextTab,
        PreviousTab => Shortcut::PreviousTab,
        FocusOmnibox => Shortcut::FocusOmnibox,
        CommandPalette => Shortcut::CommandPalette,
        ZoomIn => Shortcut::ZoomIn,
        ZoomOut => Shortcut::ZoomOut,
        ZoomReset => Shortcut::ZoomReset,
        ToggleBookmark => Shortcut::ToggleBookmark,
        Back => Shortcut::Back,
        Forward => Shortcut::Forward,
        MoveTabLeft => Shortcut::MoveTabLeft,
        BrowseHistory => Shortcut::History,
        OpenSettings => Shortcut::Settings,
        FindInPage => Shortcut::Find,
        MoveTabRight => Shortcut::MoveTabRight,
    )
}
