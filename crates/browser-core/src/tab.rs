use serde::{Deserialize, Serialize};

use crate::TabId;

pub const DEFAULT_ZOOM: f32 = 1.0;
pub const MIN_ZOOM: f32 = 0.25;
pub const MAX_ZOOM: f32 = 5.0;

#[derive(Debug, Clone, Copy, Default, PartialEq, Serialize, Deserialize)]
pub struct ScrollPosition {
    pub x: f32,
    pub y: f32,
}

/// A tab as it is saved and restored. Live engine state stays in `browser-engine`.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct SavedTab {
    pub id: TabId,
    pub parent: Option<TabId>,
    pub url: String,
    pub title: String,
    #[serde(default)]
    pub scroll: ScrollPosition,
    #[serde(default = "default_zoom")]
    pub zoom: f32,
    #[serde(default)]
    pub muted: bool,
    #[serde(default)]
    pub pinned: bool,
}

impl SavedTab {
    pub fn new(id: TabId, url: impl Into<String>, parent: Option<TabId>) -> Self {
        Self {
            id,
            parent,
            url: url.into(),
            title: String::new(),
            scroll: ScrollPosition::default(),
            zoom: DEFAULT_ZOOM,
            muted: false,
            pinned: false,
        }
    }

    pub fn set_zoom(&mut self, zoom: f32) {
        self.zoom = zoom.clamp(MIN_ZOOM, MAX_ZOOM);
    }

    pub fn display_title(&self) -> &str {
        if self.title.is_empty() {
            &self.url
        } else {
            &self.title
        }
    }
}

fn default_zoom() -> f32 {
    DEFAULT_ZOOM
}
