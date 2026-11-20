use serde::{Deserialize, Serialize};

use crate::TabId;

/// Most-recently-used tab order, front is the newest. Drives Ctrl+Tab.
#[derive(Debug, Clone, Default, PartialEq, Eq, Serialize, Deserialize)]
#[serde(transparent)]
pub struct MruOrder(Vec<TabId>);

impl MruOrder {
    pub fn touch(&mut self, id: TabId) {
        self.remove(id);
        self.0.insert(0, id);
    }

    pub fn remove(&mut self, id: TabId) {
        self.0.retain(|t| *t != id);
    }

    pub fn most_recent(&self) -> Option<TabId> {
        self.0.first().copied()
    }

    /// The tab `steps` positions back from the newest, wrapping. One Ctrl+Tab
    /// press is `steps = 1`; holding Ctrl and pressing Tab again adds a step.
    pub fn back(&self, steps: usize) -> Option<TabId> {
        if self.0.is_empty() {
            return None;
        }
        self.0.get(steps % self.0.len()).copied()
    }

    pub fn iter(&self) -> impl Iterator<Item = TabId> + '_ {
        self.0.iter().copied()
    }

    pub fn len(&self) -> usize {
        self.0.len()
    }

    pub fn is_empty(&self) -> bool {
        self.0.is_empty()
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn touch_moves_to_front() {
        let mut mru = MruOrder::default();
        mru.touch(TabId(1));
        mru.touch(TabId(2));
        mru.touch(TabId(1));
        assert_eq!(mru.iter().collect::<Vec<_>>(), vec![TabId(1), TabId(2)]);
    }

    #[test]
    fn back_wraps() {
        let mut mru = MruOrder::default();
        for id in 1..=3 {
            mru.touch(TabId(id));
        }
        assert_eq!(mru.back(1), Some(TabId(2)));
        assert_eq!(mru.back(3), Some(TabId(3)));
        assert_eq!(MruOrder::default().back(1), None);
    }
}
