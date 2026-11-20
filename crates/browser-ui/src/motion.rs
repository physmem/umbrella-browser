//! Springs for the chrome: the tab strip (each tab's x and width, the
//! gliding active capsule, closing tabs shrinking away) and a keyed store
//! any view can drive, so every animation shares the theme's spring and the
//! user's Calm/Lively choice.

use std::collections::HashMap;

use browser_core::TabId;
use browser_core::spring::{Spring, SpringParams};

#[derive(Debug, Clone, Copy, PartialEq)]
pub struct Slot {
    pub x: f32,
    pub width: f32,
}

struct TabSprings {
    x: Spring,
    width: Spring,
}

/// A closed tab still on screen while it shrinks to nothing.
pub struct Ghost {
    pub title: String,
    pub x: f32,
    width: Spring,
    from: f32,
}

impl Ghost {
    pub fn width(&self) -> f32 {
        self.width.value()
    }

    /// 1 when the tab just closed, 0 when it is gone.
    pub fn presence(&self) -> f32 {
        if self.from <= 0.0 {
            0.0
        } else {
            (self.width.value() / self.from).clamp(0.0, 1.0)
        }
    }
}

pub struct TabMotion {
    params: SpringParams,
    tabs: HashMap<TabId, TabSprings>,
    titles: HashMap<TabId, String>,
    ghosts: Vec<Ghost>,
    capsule_x: Spring,
    capsule_width: Spring,
}

impl TabMotion {
    pub fn new(params: SpringParams) -> Self {
        Self {
            params,
            tabs: HashMap::new(),
            titles: HashMap::new(),
            ghosts: Vec::new(),
            capsule_x: Spring::new(params, 0.0),
            capsule_width: Spring::new(params, 0.0),
        }
    }

    pub fn set_params(&mut self, params: SpringParams) {
        self.params = params;
        for tab in self.tabs.values_mut() {
            tab.x.params = params;
            tab.width.params = params;
        }
        self.capsule_x.params = params;
        self.capsule_width.params = params;
    }

    /// New tabs grow from zero width at their slot; existing ones glide.
    pub fn sync(&mut self, layout: &[(TabId, Slot)], active: Option<TabId>) {
        self.retire_closed(layout);
        for (id, slot) in layout {
            let params = self.params;
            let springs = self.tabs.entry(*id).or_insert_with(|| TabSprings {
                x: Spring::new(params, slot.x),
                width: Spring::new(params, 0.0),
            });
            springs.x.set_target(slot.x);
            springs.width.set_target(slot.width);
        }
        self.sync_capsule(layout, active);
    }

    /// Tabs gone from the layout become ghosts that shrink in place.
    fn retire_closed(&mut self, layout: &[(TabId, Slot)]) {
        let closed: Vec<TabId> = self
            .tabs
            .keys()
            .filter(|id| !layout.iter().any(|(t, _)| t == *id))
            .copied()
            .collect();
        for id in closed {
            let Some(springs) = self.tabs.remove(&id) else {
                continue;
            };
            let from = springs.width.value();
            let mut width = Spring::new(self.params, from);
            width.set_target(0.0);
            let title = self.titles.remove(&id).unwrap_or_default();
            self.ghosts.push(Ghost {
                title,
                x: springs.x.value(),
                width,
                from,
            });
        }
    }

    pub fn remember_title(&mut self, id: TabId, title: &str) {
        if self.titles.get(&id).is_none_or(|t| t != title) {
            self.titles.insert(id, title.to_owned());
        }
    }

    pub fn ghosts(&self) -> &[Ghost] {
        &self.ghosts
    }

    fn sync_capsule(&mut self, layout: &[(TabId, Slot)], active: Option<TabId>) {
        let Some(slot) = active
            .and_then(|a| layout.iter().find(|(t, _)| *t == a))
            .map(|(_, s)| *s)
        else {
            return;
        };
        // First placement snaps; later moves glide.
        if self.capsule_width.target() == 0.0 {
            self.capsule_x.snap_to(slot.x);
            self.capsule_width.snap_to(slot.width);
            return;
        }
        self.capsule_x.set_target(slot.x);
        self.capsule_width.set_target(slot.width);
    }

    /// Advances every spring; returns true while anything is still moving.
    pub fn step(&mut self, dt: f32, reduced_motion: bool) -> bool {
        let mut moving = false;
        for tab in self.tabs.values_mut() {
            tab.x.step(dt, reduced_motion);
            tab.width.step(dt, reduced_motion);
            moving |= !tab.x.is_settled() || !tab.width.is_settled();
        }
        for ghost in &mut self.ghosts {
            ghost.width.step(dt, reduced_motion);
        }
        self.ghosts
            .retain(|g| g.width.value() > 0.5 && !g.width.is_settled());
        moving |= !self.ghosts.is_empty();
        self.capsule_x.step(dt, reduced_motion);
        self.capsule_width.step(dt, reduced_motion);
        moving || !self.capsule_x.is_settled() || !self.capsule_width.is_settled()
    }

    pub fn slot(&self, id: TabId) -> Option<Slot> {
        let tab = self.tabs.get(&id)?;
        Some(Slot {
            x: tab.x.value(),
            width: tab.width.value(),
        })
    }

    pub fn capsule(&self) -> Slot {
        Slot {
            x: self.capsule_x.value(),
            width: self.capsule_width.value(),
        }
    }
}

/// Named springs for one-off animations: a panel's reveal, a toggle's
/// knob, the omnibox's focus. `drive` sets the target and returns the
/// current value; the browser steps them every frame while any moves.
pub struct Springs {
    params: SpringParams,
    map: HashMap<&'static str, Spring>,
    keyed: HashMap<String, Spring>,
}

impl Springs {
    pub fn new(params: SpringParams) -> Self {
        Self {
            params,
            map: HashMap::new(),
            keyed: HashMap::new(),
        }
    }

    pub fn set_params(&mut self, params: SpringParams) {
        self.params = params;
        for spring in self.map.values_mut().chain(self.keyed.values_mut()) {
            spring.params = params;
        }
    }

    /// `from` is where a brand-new spring starts, e.g. 0 to animate in.
    pub fn drive(&mut self, key: &'static str, from: f32, target: f32) -> f32 {
        let params = self.params;
        let spring = self
            .map
            .entry(key)
            .or_insert_with(|| Spring::new(params, from));
        spring.set_target(target);
        spring.value()
    }

    /// Puts `key` at `value` with no motion, e.g. a scrollbar thumb that must
    /// follow the pointer exactly.
    pub fn snap(&mut self, key: &'static str, value: f32) {
        let params = self.params;
        self.map
            .entry(key)
            .or_insert_with(|| Spring::new(params, value))
            .snap_to(value);
    }

    /// Forgets `key`, so the next `drive` starts it again from its `from`.
    /// Used to replay an entrance, e.g. when the settings pane changes.
    pub fn replay(&mut self, key: &'static str) {
        self.map.remove(key);
    }

    /// Like `drive`, for keys built at runtime (one per settings row).
    pub fn drive_keyed(&mut self, key: String, from: f32, target: f32) -> f32 {
        let params = self.params;
        let spring = self
            .keyed
            .entry(key)
            .or_insert_with(|| Spring::new(params, from));
        spring.set_target(target);
        spring.value()
    }

    pub fn step(&mut self, dt: f32, reduced_motion: bool) {
        for spring in self.map.values_mut().chain(self.keyed.values_mut()) {
            spring.step(dt, reduced_motion);
        }
    }

    pub fn moving(&self) -> bool {
        self.map
            .values()
            .chain(self.keyed.values())
            .any(|s| !s.is_settled())
    }
}

/// Even tab widths between a floor and a ceiling, left to right.
pub fn layout_tabs(ids: &[TabId], start: f32, available: f32, gap: f32) -> Vec<(TabId, Slot)> {
    const MIN: f32 = 72.0;
    const MAX: f32 = 220.0;
    if ids.is_empty() {
        return Vec::new();
    }
    let count = ids.len() as f32;
    let width = ((available - gap * (count - 1.0)) / count).clamp(MIN, MAX);
    ids.iter()
        .enumerate()
        .map(|(i, id)| {
            (
                *id,
                Slot {
                    x: start + i as f32 * (width + gap),
                    width,
                },
            )
        })
        .collect()
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn closed_tab_shrinks_as_ghost() {
        let mut motion = TabMotion::new(SpringParams::new(380.0, 40.0));
        let layout = layout_tabs(&[TabId(1), TabId(2)], 0.0, 1000.0, 0.0);
        motion.sync(&layout, Some(TabId(1)));
        for _ in 0..120 {
            motion.step(1.0 / 60.0, false);
        }
        motion.remember_title(TabId(2), "Closing");
        motion.sync(&layout[..1], Some(TabId(1)));
        assert_eq!(motion.ghosts().len(), 1);
        assert_eq!(motion.ghosts()[0].title, "Closing");
        for _ in 0..120 {
            motion.step(1.0 / 60.0, false);
        }
        assert!(motion.ghosts().is_empty());
    }

    #[test]
    fn springs_drive_toward_targets() {
        let mut springs = Springs::new(SpringParams::new(400.0, 40.0));
        assert_eq!(springs.drive("panel", 0.0, 1.0), 0.0);
        for _ in 0..120 {
            springs.step(1.0 / 60.0, false);
        }
        assert_eq!(springs.drive("panel", 0.0, 1.0), 1.0);
        assert!(!springs.moving());
    }

    #[test]
    fn layout_clamps_width() {
        let ids: Vec<TabId> = (1..=3).map(TabId).collect();
        let wide = layout_tabs(&ids, 8.0, 3000.0, 4.0);
        assert_eq!(wide[0].1.width, 220.0);
        assert_eq!(wide[1].1.x, 8.0 + 224.0);
        let narrow = layout_tabs(&ids, 0.0, 100.0, 0.0);
        assert_eq!(narrow[2].1.width, 72.0);
    }

    #[test]
    fn new_tab_grows_and_settles() {
        let mut motion = TabMotion::new(SpringParams::new(380.0, 40.0));
        let layout = layout_tabs(&[TabId(1)], 0.0, 500.0, 0.0);
        motion.sync(&layout, Some(TabId(1)));
        assert_eq!(motion.slot(TabId(1)).map(|s| s.width), Some(0.0));
        for _ in 0..120 {
            motion.step(1.0 / 60.0, false);
        }
        assert_eq!(motion.slot(TabId(1)).map(|s| s.width), Some(220.0));
        assert!(!motion.step(1.0 / 60.0, false));
    }
}
