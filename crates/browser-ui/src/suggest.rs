//! What the omnibox offers as you type. Text starting with `>` switches it
//! into the command palette, so one field and one list serve both.

use browser_core::fuzzy;
use browser_core::history::{Bookmark, Visit};
use browser_core::settings::OmniboxTarget;
use browser_core::{SavedTab, TabId, ThemeId, WorkspaceId};

pub const COMMAND_PREFIX: char = '>';
/// `#` browses history only; on its own it lists recent pages.
pub const HISTORY_PREFIX: char = '#';
const LIMIT: usize = 8;
const SWITCH_BONUS: i32 = 64;

#[derive(Debug, Clone, PartialEq)]
pub enum Command {
    NewTab,
    CloseTab,
    ReopenTab,
    ToggleBookmark,
    ToggleSplit,
    Theme(ThemeId),
    MotionCalm,
    MotionLively,
    ToggleReducedGlass,
    ToggleAdBlock,
    LockVault,
    ToggleVaultDpapi,
    UnlockVault,
    ClearHistory,
    UpdateFilterLists,
    BrowseHistory,
    OpenSettings,
    OpenDownloads,
    PageInfo,
    ReaderMode,
    FindInPage,
    PinPage,
    NewWorkspace(String),
    SwitchWorkspace(WorkspaceId),
    RenameWorkspace(String),
}

#[derive(Debug, Clone, PartialEq)]
pub enum Action {
    SwitchTab(TabId),
    Open(String),
    Run(Command),
}

#[derive(Debug, Clone, PartialEq)]
pub struct Suggestion {
    pub action: Action,
    pub title: String,
    pub detail: String,
    pub kind: Kind,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Kind {
    Go,
    Search,
    Tab,
    Bookmark,
    History,
    Command,
}

/// Sources the omnibox searches, gathered by the caller.
pub struct Sources<'a> {
    pub tabs: &'a [SavedTab],
    pub bookmarks: Vec<Bookmark>,
    pub history: Vec<Visit>,
    pub themes: Vec<(ThemeId, String)>,
    /// Every workspace except the active one.
    pub workspaces: Vec<(WorkspaceId, String)>,
    pub search_template: &'a str,
}

pub fn is_history(input: &str) -> bool {
    input.trim_start().starts_with(HISTORY_PREFIX)
}

/// The query after a `#`, or `None` outside history mode.
pub fn history_query(input: &str) -> Option<&str> {
    input
        .trim_start()
        .strip_prefix(HISTORY_PREFIX)
        .map(str::trim)
}

pub fn is_command(input: &str) -> bool {
    input.trim_start().starts_with(COMMAND_PREFIX)
}

pub fn suggest(input: &str, sources: &Sources) -> Vec<Suggestion> {
    if let Some(query) = input.trim_start().strip_prefix(COMMAND_PREFIX) {
        return commands(query.trim(), sources);
    }
    if is_history(input) {
        return sources.history.iter().map(visit).collect();
    }
    let query = input.trim();
    if query.is_empty() {
        return Vec::new();
    }
    let mut out = vec![direct(query, sources.search_template)];
    out.extend(matching_tabs(query, sources.tabs));
    let open = |url: &str| sources.tabs.iter().any(|t| t.url == url);
    // An open tab already offers "Switch to tab"; don't list its page twice.
    out.extend(
        sources
            .bookmarks
            .iter()
            .filter(|b| !open(&b.url))
            .map(bookmark),
    );
    out.extend(
        sources
            .history
            .iter()
            .filter(|v| !open(&v.url) && !out_has(&out, &v.url))
            .map(visit)
            .collect::<Vec<_>>(),
    );
    out.dedup_by(|a, b| a.action == b.action);
    out.truncate(LIMIT);
    out
}

fn direct(query: &str, template: &str) -> Suggestion {
    match OmniboxTarget::interpret(query) {
        Some(OmniboxTarget::Url(url)) => Suggestion {
            title: url.clone(),
            detail: "Go to address".into(),
            action: Action::Open(url),
            kind: Kind::Go,
        },
        _ => Suggestion {
            title: query.to_owned(),
            detail: "Search".into(),
            action: Action::Open(OmniboxTarget::Search(query.to_owned()).into_url(template)),
            kind: Kind::Search,
        },
    }
}

fn matching_tabs(query: &str, tabs: &[SavedTab]) -> Vec<Suggestion> {
    let mut hits: Vec<(i32, &SavedTab)> = tabs
        .iter()
        .filter_map(|t| Some((fuzzy::best(query, &[&t.title, &t.url])?, t)))
        .collect();
    hits.sort_by_key(|(score, _)| std::cmp::Reverse(*score));
    hits.into_iter()
        .take(3)
        .map(|(_, t)| Suggestion {
            action: Action::SwitchTab(t.id),
            title: t.display_title().to_owned(),
            detail: "Switch to tab".into(),
            kind: Kind::Tab,
        })
        .collect()
}

fn bookmark(mark: &Bookmark) -> Suggestion {
    Suggestion {
        action: Action::Open(mark.url.clone()),
        title: if mark.title.is_empty() {
            mark.url.clone()
        } else {
            mark.title.clone()
        },
        detail: mark.url.clone(),
        kind: Kind::Bookmark,
    }
}

fn visit(v: &Visit) -> Suggestion {
    Suggestion {
        action: Action::Open(v.url.clone()),
        title: if v.title.is_empty() {
            v.url.clone()
        } else {
            v.title.clone()
        },
        detail: v.url.clone(),
        kind: Kind::History,
    }
}

fn out_has(out: &[Suggestion], url: &str) -> bool {
    out.iter().any(|s| s.action == Action::Open(url.to_owned()))
}

fn commands(query: &str, sources: &Sources) -> Vec<Suggestion> {
    let mut all = fixed_commands();
    all.extend(
        sources
            .themes
            .iter()
            .map(|(id, name)| (Command::Theme(id.clone()), format!("Theme: {name}"))),
    );
    all.extend(sources.workspaces.iter().map(|(id, name)| {
        (
            Command::SwitchWorkspace(*id),
            format!("Switch to workspace: {name}"),
        )
    }));
    // Commands that take a name read it from the rest of the query.
    let named = |prefix: &str| {
        let rest = query
            .get(..prefix.len())?
            .eq_ignore_ascii_case(prefix)
            .then(|| query[prefix.len()..].trim())?;
        (!rest.is_empty()).then(|| rest.to_owned())
    };
    if let Some(name) = named("rename workspace") {
        return vec![command_row(
            Command::RenameWorkspace(name.clone()),
            format!("Rename workspace to \u{201c}{name}\u{201d}"),
        )];
    }
    if let Some(name) = named("new workspace") {
        return vec![command_row(
            Command::NewWorkspace(name.clone()),
            format!("New workspace \u{201c}{name}\u{201d}"),
        )];
    }
    let mut hits: Vec<(i32, Suggestion)> = all
        .into_iter()
        .filter_map(|(command, title)| {
            // Going to an existing workspace beats making another one.
            let bonus = if matches!(command, Command::SwitchWorkspace(_)) {
                SWITCH_BONUS
            } else {
                0
            };
            let score = fuzzy::score(query, &title)? + bonus;
            Some((
                score,
                Suggestion {
                    action: Action::Run(command),
                    title,
                    detail: String::new(),
                    kind: Kind::Command,
                },
            ))
        })
        .collect();
    hits.sort_by_key(|(score, _)| std::cmp::Reverse(*score));
    hits.into_iter().take(LIMIT * 2).map(|(_, s)| s).collect()
}

fn command_row(command: Command, title: String) -> Suggestion {
    Suggestion {
        action: Action::Run(command),
        title,
        detail: String::new(),
        kind: Kind::Command,
    }
}

fn fixed_commands() -> Vec<(Command, String)> {
    [
        (Command::NewTab, "New tab"),
        (Command::CloseTab, "Close tab"),
        (Command::ReopenTab, "Reopen closed tab"),
        (Command::ToggleBookmark, "Bookmark this page"),
        (Command::ToggleSplit, "Toggle split view"),
        (Command::MotionCalm, "Motion: Calm"),
        (Command::MotionLively, "Motion: Lively"),
        (Command::ToggleReducedGlass, "Toggle reduced glass"),
        (Command::ToggleAdBlock, "Toggle ad blocking"),
        (Command::LockVault, "Lock password vault"),
        (
            Command::ToggleVaultDpapi,
            "Toggle vault unlock with Windows",
        ),
        (Command::UnlockVault, "Unlock password vault"),
        (Command::ClearHistory, "Clear browsing history"),
        (Command::UpdateFilterLists, "Update ad-block filter lists"),
        (Command::BrowseHistory, "Browse history"),
        (Command::OpenSettings, "Settings"),
        (Command::OpenDownloads, "Downloads"),
        (Command::PageInfo, "Page info"),
        (Command::ReaderMode, "Reader view"),
        (Command::FindInPage, "Find on page"),
        (Command::PinPage, "Pin page to start page"),
        (Command::NewWorkspace(String::new()), "New workspace"),
    ]
    .into_iter()
    .map(|(c, t)| (c, t.to_owned()))
    .collect()
}

#[cfg(test)]
mod tests {
    use super::*;

    fn sources<'a>(tabs: &'a [SavedTab]) -> Sources<'a> {
        Sources {
            tabs,
            bookmarks: Vec::new(),
            history: vec![Visit {
                url: "https://doc.rust-lang.org/".into(),
                title: "Rust Docs".into(),
                visits: 3,
                last_visit: 0,
            }],
            themes: vec![(ThemeId::new("nord"), "Nord".into())],
            workspaces: vec![(WorkspaceId(2), "Work".into())],
            search_template: "https://s.test/?q={query}",
        }
    }

    #[test]
    fn first_row_is_go_or_search() {
        let s = suggest("rust docs", &sources(&[]));
        assert_eq!(s[0].kind, Kind::Search);
        assert_eq!(
            s[0].action,
            Action::Open("https://s.test/?q=rust+docs".into())
        );
        let s = suggest("example.com", &sources(&[]));
        assert_eq!(s[0].action, Action::Open("https://example.com".into()));
    }

    #[test]
    fn open_tabs_and_history_match() {
        let mut tab = SavedTab::new(TabId(4), "https://github.com/", None);
        tab.title = "GitHub".into();
        let tabs = [tab];
        let s = suggest("git", &sources(&tabs));
        assert!(s.iter().any(|x| x.action == Action::SwitchTab(TabId(4))));
        let s = suggest("rust", &sources(&tabs));
        assert!(s.iter().any(|x| x.kind == Kind::History));
    }

    #[test]
    fn hash_lists_history_only() {
        let s = suggest("#", &sources(&[]));
        assert_eq!(s.len(), 1);
        assert_eq!(s[0].kind, Kind::History);
        assert_eq!(history_query("#  rust "), Some("rust"));
    }

    #[test]
    fn workspace_commands() {
        let s = suggest(">workspace", &sources(&[]));
        assert_eq!(
            s[0].action,
            Action::Run(Command::SwitchWorkspace(WorkspaceId(2)))
        );
        let s = suggest(">rename workspace Research", &sources(&[]));
        assert_eq!(
            s[0].action,
            Action::Run(Command::RenameWorkspace("Research".into()))
        );
        let s = suggest(">new workspace Play", &sources(&[]));
        assert_eq!(
            s[0].action,
            Action::Run(Command::NewWorkspace("Play".into()))
        );
    }

    #[test]
    fn prefix_switches_to_commands() {
        let s = suggest(">nord", &sources(&[]));
        assert_eq!(
            s[0].action,
            Action::Run(Command::Theme(ThemeId::new("nord")))
        );
        let s = suggest(">", &sources(&[]));
        assert!(s.len() > 10);
        assert!(s.iter().all(|x| x.kind == Kind::Command));
    }
}
