use serde::{Deserialize, Serialize};

use crate::mru::MruOrder;
use crate::{SavedTab, TabId, ThemeId, WorkspaceId};

/// How many closed tabs Ctrl+Shift+T can bring back.
pub const CLOSED_TAB_LIMIT: usize = 25;

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct ClosedTab {
    pub tab: SavedTab,
    pub index: usize,
}

/// A named set of tabs. Tabs are stored in strip order; `parent` links form
/// the tree, and a child always sits after its parent in the strip.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct Workspace {
    pub id: WorkspaceId,
    pub name: String,
    pub tabs: Vec<SavedTab>,
    pub active: Option<TabId>,
    #[serde(default)]
    pub mru: MruOrder,
    #[serde(default)]
    pub closed: Vec<ClosedTab>,
    #[serde(default)]
    pub theme: Option<ThemeId>,
}

impl Workspace {
    pub fn new(id: WorkspaceId, name: impl Into<String>) -> Self {
        Self {
            id,
            name: name.into(),
            tabs: Vec::new(),
            active: None,
            mru: MruOrder::default(),
            closed: Vec::new(),
            theme: None,
        }
    }

    pub fn tab(&self, id: TabId) -> Option<&SavedTab> {
        self.tabs.iter().find(|t| t.id == id)
    }

    pub fn tab_mut(&mut self, id: TabId) -> Option<&mut SavedTab> {
        self.tabs.iter_mut().find(|t| t.id == id)
    }

    pub fn active_tab(&self) -> Option<&SavedTab> {
        self.tab(self.active?)
    }

    pub fn children(&self, id: TabId) -> impl Iterator<Item = &SavedTab> {
        self.tabs.iter().filter(move |t| t.parent == Some(id))
    }

    /// Inserts a tab after its parent's subtree, or at the end without a parent.
    pub fn insert_tab(&mut self, tab: SavedTab) {
        let index = match tab.parent.and_then(|p| self.index_of(p)) {
            Some(parent_index) => self.subtree_end(parent_index),
            None => self.tabs.len(),
        };
        self.tabs.insert(index, tab);
    }

    pub fn activate(&mut self, id: TabId) -> bool {
        if self.tab(id).is_none() {
            return false;
        }
        self.active = Some(id);
        self.mru.touch(id);
        true
    }

    /// Removes a tab, lifts its children to its parent, and remembers it for
    /// Ctrl+Shift+T. Activates the most recently used remaining tab.
    pub fn close_tab(&mut self, id: TabId) -> Option<SavedTab> {
        let index = self.index_of(id)?;
        let tab = self.tabs.remove(index);
        self.reparent_children(id, tab.parent);
        self.mru.remove(id);
        self.remember_closed(tab.clone(), index);
        if self.active == Some(id) {
            self.active = self.mru.most_recent().or_else(|| self.neighbor(index));
        }
        Some(tab)
    }

    pub fn reopen_closed(&mut self) -> Option<TabId> {
        let ClosedTab { mut tab, index } = self.closed.pop()?;
        if tab.parent.is_some_and(|p| self.tab(p).is_none()) {
            tab.parent = None;
        }
        let id = tab.id;
        self.tabs.insert(index.min(self.tabs.len()), tab);
        self.activate(id);
        Some(id)
    }

    /// Moves a tab to `index` in strip order (clamped). A moved tab leaves its
    /// tree: the user placed it, so it no longer follows its opener.
    pub fn move_tab(&mut self, id: TabId, index: usize) -> bool {
        let Some(from) = self.index_of(id) else {
            return false;
        };
        let mut tab = self.tabs.remove(from);
        tab.parent = None;
        self.tabs.insert(index.min(self.tabs.len()), tab);
        true
    }

    /// Position of a tab in strip order.
    pub fn position(&self, id: TabId) -> Option<usize> {
        self.index_of(id)
    }

    fn index_of(&self, id: TabId) -> Option<usize> {
        self.tabs.iter().position(|t| t.id == id)
    }

    fn subtree_end(&self, parent_index: usize) -> usize {
        let root = self.tabs[parent_index].id;
        let mut end = parent_index + 1;
        while end < self.tabs.len() && self.is_descendant(self.tabs[end].id, root) {
            end += 1;
        }
        end
    }

    fn is_descendant(&self, id: TabId, ancestor: TabId) -> bool {
        let mut current = self.tab(id).and_then(|t| t.parent);
        // Bounded walk guards against a parent cycle in a hand-edited session.
        for _ in 0..self.tabs.len() {
            match current {
                Some(p) if p == ancestor => return true,
                Some(p) => current = self.tab(p).and_then(|t| t.parent),
                None => return false,
            }
        }
        false
    }

    fn reparent_children(&mut self, id: TabId, new_parent: Option<TabId>) {
        for tab in self.tabs.iter_mut().filter(|t| t.parent == Some(id)) {
            tab.parent = new_parent;
        }
    }

    fn remember_closed(&mut self, tab: SavedTab, index: usize) {
        self.closed.push(ClosedTab { tab, index });
        if self.closed.len() > CLOSED_TAB_LIMIT {
            self.closed.remove(0);
        }
    }

    fn neighbor(&self, index: usize) -> Option<TabId> {
        let index = index.min(self.tabs.len().checked_sub(1)?);
        Some(self.tabs[index].id)
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn workspace_with(ids: &[(u64, Option<u64>)]) -> Workspace {
        let mut ws = Workspace::new(WorkspaceId(1), "Main");
        for (id, parent) in ids {
            let tab = SavedTab::new(TabId(*id), format!("https://{id}.test/"), parent.map(TabId));
            ws.insert_tab(tab);
        }
        ws
    }

    fn order(ws: &Workspace) -> Vec<u64> {
        ws.tabs.iter().map(|t| t.id.0).collect()
    }

    #[test]
    fn child_goes_after_parent_subtree() {
        let ws = workspace_with(&[
            (1, None),
            (2, None),
            (3, Some(1)),
            (4, Some(3)),
            (5, Some(1)),
        ]);
        assert_eq!(order(&ws), vec![1, 3, 4, 5, 2]);
    }

    #[test]
    fn close_lifts_children_and_activates_mru() {
        let mut ws = workspace_with(&[(1, None), (2, Some(1)), (3, None)]);
        ws.activate(TabId(3));
        ws.activate(TabId(1));
        ws.close_tab(TabId(1));
        assert_eq!(ws.tab(TabId(2)).and_then(|t| t.parent), None);
        assert_eq!(ws.active, Some(TabId(3)));
    }

    #[test]
    fn reopen_restores_position() {
        let mut ws = workspace_with(&[(1, None), (2, None), (3, None)]);
        ws.close_tab(TabId(2));
        assert_eq!(ws.reopen_closed(), Some(TabId(2)));
        assert_eq!(order(&ws), vec![1, 2, 3]);
        assert_eq!(ws.active, Some(TabId(2)));
    }

    #[test]
    fn move_tab_reorders_and_detaches() {
        let mut ws = workspace_with(&[(1, None), (2, Some(1)), (3, None)]);
        assert!(ws.move_tab(TabId(2), 9));
        assert_eq!(order(&ws), vec![1, 3, 2]);
        assert_eq!(ws.tab(TabId(2)).and_then(|t| t.parent), None);
        assert!(ws.move_tab(TabId(3), 0));
        assert_eq!(order(&ws), vec![3, 1, 2]);
        assert!(!ws.move_tab(TabId(9), 0));
    }

    #[test]
    fn closing_last_tab_clears_active() {
        let mut ws = workspace_with(&[(1, None)]);
        ws.activate(TabId(1));
        ws.close_tab(TabId(1));
        assert_eq!(ws.active, None);
    }
}
