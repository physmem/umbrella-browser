//! Omnibox behavior: suggestions while typing, Up/Down selection, and the
//! `>` command palette.
//!
//! The list is drawn inside the main window. GPUI cannot paint over a
//! WebView2 page, so while the list is open the page view is hidden and the
//! list takes the page area on the window glass (see `chrome.rs`). A separate
//! popup window was tried first and fought window activation.

use gpui::{Context, Window};

use crate::browser::Browser;
use crate::suggest::{self, Action, Command, Sources, Suggestion};

const HISTORY_ROWS: usize = 5;
const BOOKMARK_ROWS: usize = 3;
const HISTORY_PAGE: usize = 14;

impl Browser {
    pub(crate) fn refresh_suggestions(&mut self, cx: &mut Context<Self>) {
        let text = self.omnibox.read(cx).as_str().to_owned();
        // Focusing the omnibox fills it with the current URL; that is not a
        // query yet, so no list until the user types.
        let is_current_url = self.active_tab().is_some_and(|t| t.url == text);
        self.suggestions = if is_current_url {
            Vec::new()
        } else {
            self.suggestions_for(&text)
        };
        self.selected = 0;
        self.sync_views();
        cx.notify();
    }

    pub(crate) fn suggestions_open(&self) -> bool {
        self.omnibox_editing && !self.suggestions.is_empty()
    }

    fn suggestions_for(&self, text: &str) -> Vec<Suggestion> {
        let tabs = self
            .workspace()
            .map(|w| w.tabs.as_slice())
            .unwrap_or_default();
        let query = text.trim();
        let command = suggest::is_command(text);
        let bookmarks = if command {
            Vec::new()
        } else {
            self.bookmarks
                .search(query, BOOKMARK_ROWS)
                .unwrap_or_default()
        };
        let now = crate::browser::unix_now();
        let history = match suggest::history_query(text) {
            Some("") => self.history.recent(HISTORY_PAGE).unwrap_or_default(),
            Some(q) => self
                .history
                .search(q, now, HISTORY_PAGE)
                .unwrap_or_default(),
            None if command => Vec::new(),
            None => self
                .history
                .search(query, now, HISTORY_ROWS)
                .unwrap_or_default(),
        };
        let sources = Sources {
            tabs,
            bookmarks,
            history,
            themes: self.theme_names(),
            workspaces: self.other_workspaces(),
            search_template: &self.settings.search_template,
        };
        suggest::suggest(text, &sources)
    }

    fn theme_names(&self) -> Vec<(browser_core::ThemeId, String)> {
        self.themes
            .ids()
            .filter_map(|id| Some((id.clone(), self.themes.get(id)?.name.clone())))
            .collect()
    }

    /// GPUI reports a blur whenever the window turns inactive. Alt-Tab away
    /// and back should find the omnibox as it was, so only a blur inside an
    /// active window ends editing.
    pub(crate) fn on_omnibox_blur(&mut self, window: &mut Window, cx: &mut Context<Self>) {
        if !window.is_window_active() {
            return;
        }
        self.omnibox_editing = false;
        self.close_suggestions(cx);
        self.sync_views();
        // With nothing focused, window shortcuts like Ctrl+L stop matching;
        // the root keeps them alive until the page takes focus.
        window.focus(&self.focus, cx);
        cx.notify();
    }

    /// Leaves the address field without touching focus, for clicks that
    /// land somewhere a blur never reports (the page, blank chrome).
    pub(crate) fn end_omnibox_editing(&mut self, cx: &mut Context<Self>) {
        if !self.omnibox_editing {
            return;
        }
        self.omnibox_editing = false;
        self.suggestions.clear();
        self.sync_views();
        cx.notify();
    }

    /// A press on chrome outside the address field ends editing and parks
    /// focus on the root so window shortcuts keep working.
    pub(crate) fn press_outside_omnibox(
        &mut self,
        position: gpui::Point<gpui::Pixels>,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) {
        let (x, y) = (f32::from(position.x), f32::from(position.y));
        let (left, width) = self.omnibox_span;
        let in_toolbar = (crate::style::TAB_ROW..crate::style::CHROME_HEIGHT).contains(&y);
        if !self.omnibox_editing || (in_toolbar && (left..left + width).contains(&x)) {
            return;
        }
        self.end_omnibox_editing(cx);
        window.focus(&self.focus, cx);
    }

    pub(crate) fn close_suggestions(&mut self, cx: &mut Context<Self>) {
        if self.suggestions.is_empty() {
            return;
        }
        self.suggestions.clear();
        self.sync_views();
        cx.notify();
    }

    /// Moves the highlight; false when there is no list, so the field keeps
    /// its own Up/Down behavior.
    pub(crate) fn move_selection(&mut self, down: bool, cx: &mut Context<Self>) -> bool {
        let len = self.suggestions.len();
        if len == 0 {
            return false;
        }
        self.selected = if down {
            (self.selected + 1) % len
        } else {
            (self.selected + len - 1) % len
        };
        cx.notify();
        true
    }

    /// Enter: run the highlighted suggestion, or navigate to the typed text.
    pub(crate) fn submit_omnibox(&mut self, window: &mut Window, cx: &mut Context<Self>) {
        if self.suggestions.get(self.selected).is_some() {
            self.accept_suggestion(cx);
            return;
        }
        let text = self.omnibox.read(cx).as_str().to_owned();
        self.omnibox_editing = false;
        self.close_suggestions(cx);
        self.navigate_active(&text);
        window.blur(cx);
        cx.notify();
    }

    pub(crate) fn choose_suggestion(&mut self, index: usize, cx: &mut Context<Self>) {
        self.selected = index;
        self.accept_suggestion(cx);
    }

    pub(crate) fn accept_suggestion(&mut self, cx: &mut Context<Self>) {
        let Some(item) = self.suggestions.get(self.selected).cloned() else {
            return;
        };
        self.leave_omnibox(cx);
        match item.action {
            Action::SwitchTab(id) => self.activate_tab(id),
            Action::Open(url) => self.navigate_active(&url),
            Action::Run(command) => self.run_command(command, cx),
        }
        cx.notify();
    }

    pub(crate) fn leave_omnibox(&mut self, cx: &mut Context<Self>) {
        self.omnibox_editing = false;
        self.close_suggestions(cx);
        self.sync_views();
        let handle = self.window;
        cx.defer(move |cx| {
            let _ = handle.update(cx, |_, window, cx| window.blur(cx));
        });
        self.focus_page();
    }

    pub(crate) fn open_palette(&mut self, window: &mut Window, cx: &mut Context<Self>) {
        self.open_omnibox_with(suggest::COMMAND_PREFIX, window, cx);
    }

    pub(crate) fn open_history(&mut self, window: &mut Window, cx: &mut Context<Self>) {
        self.open_omnibox_with(suggest::HISTORY_PREFIX, window, cx);
    }

    fn open_omnibox_with(&mut self, prefix: char, window: &mut Window, cx: &mut Context<Self>) {
        self.focus_omnibox(window, cx);
        let prefix = prefix.to_string();
        self.omnibox
            .update(cx, |field, cx| field.emplace(&prefix, cx));
    }

    pub(crate) fn run_command(&mut self, command: Command, cx: &mut Context<Self>) {
        use browser_core::spring::MotionStyle;
        match command {
            Command::NewTab => self.open_tab(crate::browser::NEW_TAB_URL, None, true),
            Command::CloseTab => {
                if let Some(id) = self.active_id() {
                    self.close_tab(id);
                }
            }
            Command::ReopenTab => self.reopen_last(),
            Command::ToggleBookmark => self.toggle_bookmark(),
            Command::ToggleSplit => self.toggle_split(),
            Command::Theme(id) => self.update_settings(cx, |s| s.theme = Some(id)),
            Command::MotionCalm => self.update_settings(cx, |s| s.motion = MotionStyle::Calm),
            Command::MotionLively => self.update_settings(cx, |s| s.motion = MotionStyle::Lively),
            Command::ToggleReducedGlass => {
                self.update_settings(cx, |s| s.reduced_glass = !s.reduced_glass)
            }
            Command::ToggleAdBlock => self.update_settings(cx, |s| s.block_ads = !s.block_ads),
            Command::LockVault => self.lock_vault(),
            Command::ToggleVaultDpapi => {
                self.update_settings(cx, |s| s.vault_dpapi = !s.vault_dpapi);
                self.vault.set_dpapi(self.settings.vault_dpapi);
            }
            Command::UnlockVault => self.request_unlock(cx),
            Command::UpdateFilterLists => self.load_filter_lists(true, cx),
            Command::OpenSettings => self.open_settings(cx),
            Command::OpenDownloads => self.open_downloads(cx),
            Command::PageInfo => self.open_page_info(cx),
            Command::ReaderMode => self.toggle_reader(cx),
            Command::FindInPage => {
                self.defer_in_window(cx, |b, window, cx| b.open_find(window, cx))
            }
            Command::PinPage => self.pin_current_page(cx),
            Command::BrowseHistory => {
                self.defer_in_window(cx, |b, window, cx| b.open_history(window, cx))
            }
            Command::NewWorkspace(name) => self.new_workspace(name),
            Command::SwitchWorkspace(id) => self.switch_workspace(id),
            Command::RenameWorkspace(name) => self.rename_workspace(name),
            Command::ClearHistory => {
                let _ = self.history.clear();
            }
        }
    }
}
