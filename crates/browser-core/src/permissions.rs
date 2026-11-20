//! Remembered answers to "this site wants to use your camera" and friends,
//! keyed by exact origin so a decision never leaks to another site.

use serde::{Deserialize, Serialize};

use crate::session::SessionError;
use crate::store::KvStore;

const TABLE: &str = "permissions";

#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, Serialize, Deserialize)]
#[serde(rename_all = "kebab-case")]
pub enum PermissionKind {
    Camera,
    Microphone,
    Location,
    Notifications,
    ClipboardRead,
    Sensors,
    Midi,
    WindowManagement,
    LocalFonts,
    MultipleDownloads,
    FileEditing,
}

impl PermissionKind {
    /// Finishes "example.com wants to …".
    pub fn request_text(self) -> &'static str {
        match self {
            Self::Camera => "use your camera",
            Self::Microphone => "use your microphone",
            Self::Location => "know your location",
            Self::Notifications => "show notifications",
            Self::ClipboardRead => "read your clipboard",
            Self::Sensors => "use motion sensors",
            Self::Midi => "control MIDI devices",
            Self::WindowManagement => "manage windows on your screens",
            Self::LocalFonts => "use fonts installed on this PC",
            Self::MultipleDownloads => "download several files",
            Self::FileEditing => "edit files on this PC",
        }
    }

    pub fn label(self) -> &'static str {
        match self {
            Self::Camera => "Camera",
            Self::Microphone => "Microphone",
            Self::Location => "Location",
            Self::Notifications => "Notifications",
            Self::ClipboardRead => "Clipboard",
            Self::Sensors => "Motion sensors",
            Self::Midi => "MIDI devices",
            Self::WindowManagement => "Window management",
            Self::LocalFonts => "Local fonts",
            Self::MultipleDownloads => "Multiple downloads",
            Self::FileEditing => "File editing",
        }
    }
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct SiteDecision {
    pub origin: String,
    pub kind: PermissionKind,
    pub allow: bool,
}

#[derive(Clone)]
pub struct SitePermissions {
    kv: KvStore,
}

impl SitePermissions {
    pub fn new(kv: KvStore) -> Self {
        Self { kv }
    }

    pub fn get(&self, origin: &str, kind: PermissionKind) -> Option<bool> {
        let bytes = self.kv.get(TABLE, &key(origin, kind)).ok()??;
        let decision: SiteDecision = serde_json::from_slice(&bytes).ok()?;
        Some(decision.allow)
    }

    pub fn set(&self, origin: &str, kind: PermissionKind, allow: bool) -> Result<(), SessionError> {
        let decision = SiteDecision {
            origin: origin.to_owned(),
            kind,
            allow,
        };
        self.kv
            .put(TABLE, &key(origin, kind), &serde_json::to_vec(&decision)?)?;
        Ok(())
    }

    pub fn forget(&self, origin: &str, kind: PermissionKind) -> Result<(), SessionError> {
        self.kv.delete(TABLE, &key(origin, kind))?;
        Ok(())
    }

    /// Every remembered decision, sorted by site.
    pub fn all(&self) -> Vec<SiteDecision> {
        let mut list: Vec<SiteDecision> = self
            .kv
            .values(TABLE)
            .unwrap_or_default()
            .iter()
            .filter_map(|bytes| serde_json::from_slice(bytes).ok())
            .collect();
        list.sort_by(|a, b| {
            a.origin
                .cmp(&b.origin)
                .then(a.kind.label().cmp(b.kind.label()))
        });
        list
    }
}

fn key(origin: &str, kind: PermissionKind) -> String {
    format!("{origin}|{}", kind.label())
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn decisions_are_per_origin() -> Result<(), Box<dyn std::error::Error>> {
        let dir = tempfile::tempdir()?;
        let perms = SitePermissions::new(KvStore::open(&dir.path().join("p.redb"))?);
        perms.set("https://meet.example", PermissionKind::Camera, true)?;
        perms.set("https://ads.example", PermissionKind::Location, false)?;
        assert_eq!(
            perms.get("https://meet.example", PermissionKind::Camera),
            Some(true)
        );
        assert_eq!(
            perms.get("https://evil.example", PermissionKind::Camera),
            None
        );
        assert_eq!(
            perms.get("https://meet.example", PermissionKind::Microphone),
            None
        );
        assert_eq!(perms.all().len(), 2);
        perms.forget("https://meet.example", PermissionKind::Camera)?;
        assert_eq!(
            perms.get("https://meet.example", PermissionKind::Camera),
            None
        );
        Ok(())
    }
}
