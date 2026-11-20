use std::time::{Duration, Instant};

use browser_core::TabId;

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Residency {
    Live,
    Suspended,
}

/// What the policy needs to know about one tab that has a controller.
#[derive(Debug, Clone, Copy)]
pub struct TabActivity {
    pub id: TabId,
    pub residency: Residency,
    pub visible: bool,
    pub playing_audio: bool,
    /// When the tab was last hidden; `None` while visible.
    pub hidden_since: Option<Instant>,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum HibernateAction {
    /// `TrySuspend`: renderer frozen, memory trimmed, state kept.
    Suspend(TabId),
    /// Close the controller; the tab comes back from its `SavedTab`.
    Discard(TabId),
}

#[derive(Debug, Clone, Copy)]
pub struct HibernatePolicy {
    pub suspend_after: Duration,
    pub discard_after: Duration,
}

impl Default for HibernatePolicy {
    fn default() -> Self {
        Self {
            suspend_after: Duration::from_secs(5 * 60),
            discard_after: Duration::from_secs(60 * 60),
        }
    }
}

impl HibernatePolicy {
    pub fn plan(&self, tabs: &[TabActivity], now: Instant) -> Vec<HibernateAction> {
        tabs.iter()
            .filter_map(|tab| self.action_for(tab, now))
            .collect()
    }

    fn action_for(&self, tab: &TabActivity, now: Instant) -> Option<HibernateAction> {
        if tab.visible || tab.playing_audio {
            return None;
        }
        let hidden_for = now.saturating_duration_since(tab.hidden_since?);
        if hidden_for >= self.discard_after {
            return Some(HibernateAction::Discard(tab.id));
        }
        let should_suspend = tab.residency == Residency::Live && hidden_for >= self.suspend_after;
        should_suspend.then_some(HibernateAction::Suspend(tab.id))
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn tab(id: u64, hidden_mins: Option<u64>, now: Instant) -> TabActivity {
        TabActivity {
            id: TabId(id),
            residency: Residency::Live,
            visible: hidden_mins.is_none(),
            playing_audio: false,
            hidden_since: hidden_mins.map(|m| now - Duration::from_secs(m * 60)),
        }
    }

    #[test]
    fn suspends_then_discards_by_idle_time() {
        let now = Instant::now() + Duration::from_secs(10_000);
        let tabs = [
            tab(1, None, now),
            tab(2, Some(1), now),
            tab(3, Some(10), now),
            tab(4, Some(90), now),
        ];
        let plan = HibernatePolicy::default().plan(&tabs, now);
        assert_eq!(
            plan,
            vec![
                HibernateAction::Suspend(TabId(3)),
                HibernateAction::Discard(TabId(4))
            ]
        );
    }

    #[test]
    fn spares_audio_and_already_suspended() {
        let now = Instant::now() + Duration::from_secs(10_000);
        let mut playing = tab(1, Some(30), now);
        playing.playing_audio = true;
        let mut asleep = tab(2, Some(30), now);
        asleep.residency = Residency::Suspended;
        assert!(
            HibernatePolicy::default()
                .plan(&[playing, asleep], now)
                .is_empty()
        );
    }
}
