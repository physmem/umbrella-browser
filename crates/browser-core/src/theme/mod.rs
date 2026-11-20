//! Themes are small token sets. A user file in `themes/` may extend a preset
//! and override only the tokens it names.

pub mod color;
pub mod presets;
pub mod tokens;

use std::collections::BTreeMap;
use std::fs;
use std::path::{Path, PathBuf};

use serde_json::Value;
use thiserror::Error;

pub use color::Rgba;
pub use tokens::{Appearance, Backdrop, ThemeTokens};

use crate::ThemeId;

#[derive(Debug, Error)]
pub enum ThemeError {
    #[error("cannot read theme file: {0}")]
    Io(#[from] std::io::Error),
    #[error("invalid TOML: {0}")]
    Toml(#[from] toml::de::Error),
    #[error("invalid JSON or tokens: {0}")]
    Json(#[from] serde_json::Error),
    #[error("`extends` names unknown theme `{0}`")]
    UnknownBase(String),
    #[error("`{0}` is a built-in theme id; extend it under another file name")]
    ReservedId(ThemeId),
    #[error("theme files must end in .toml or .json")]
    UnsupportedFormat,
    #[error("{0}")]
    Invalid(&'static str),
}

#[derive(Debug)]
pub struct ThemeLoadError {
    pub path: PathBuf,
    pub error: ThemeError,
}

pub struct ThemeRegistry {
    themes: BTreeMap<ThemeId, ThemeTokens>,
}

impl ThemeRegistry {
    pub fn with_presets() -> Self {
        Self {
            themes: presets::all().into_iter().collect(),
        }
    }

    pub fn get(&self, id: &ThemeId) -> Option<&ThemeTokens> {
        self.themes.get(id)
    }

    /// Built-ins first in their own order, then custom themes by name.
    pub fn ids(&self) -> impl Iterator<Item = &ThemeId> {
        let presets: Vec<ThemeId> = presets::all().into_iter().map(|(id, _)| id).collect();
        let mut ids: Vec<&ThemeId> = self.themes.keys().collect();
        ids.sort_by_key(|id| {
            let rank = presets.iter().position(|p| p == *id).unwrap_or(usize::MAX);
            let name = self
                .themes
                .get(*id)
                .map(|t| t.name.to_lowercase())
                .unwrap_or_default();
            (rank, name)
        });
        ids.into_iter()
    }

    pub fn is_preset(id: &ThemeId) -> bool {
        presets::all().iter().any(|(preset, _)| preset == id)
    }

    /// Writes a custom theme to `dir/<id>.toml` and makes it available.
    /// Built-in ids are refused so a preset can never be overwritten.
    pub fn save_custom(
        &mut self,
        dir: &Path,
        id: &ThemeId,
        tokens: ThemeTokens,
    ) -> Result<PathBuf, ThemeError> {
        if Self::is_preset(id) {
            return Err(ThemeError::ReservedId(id.clone()));
        }
        tokens.validate()?;
        fs::create_dir_all(dir)?;
        let path = dir.join(format!("{}.toml", id.as_str()));
        let text = toml::to_string_pretty(&tokens)
            .map_err(|_| ThemeError::Invalid("theme could not be written as TOML"))?;
        fs::write(&path, text)?;
        self.themes.insert(id.clone(), tokens);
        Ok(path)
    }

    /// Changes a custom theme in memory only, e.g. while a slider drags;
    /// `save_custom` writes it once the edit settles.
    pub fn put(&mut self, id: &ThemeId, tokens: ThemeTokens) -> Result<(), ThemeError> {
        if Self::is_preset(id) {
            return Err(ThemeError::ReservedId(id.clone()));
        }
        tokens.validate()?;
        self.themes.insert(id.clone(), tokens);
        Ok(())
    }

    /// Deletes a custom theme's file and forgets it. Presets stay.
    pub fn delete_custom(&mut self, dir: &Path, id: &ThemeId) -> Result<(), ThemeError> {
        if Self::is_preset(id) {
            return Err(ThemeError::ReservedId(id.clone()));
        }
        for ext in ["toml", "json"] {
            let path = dir.join(format!("{}.{ext}", id.as_str()));
            if path.exists() {
                fs::remove_file(path)?;
            }
        }
        self.themes.remove(id);
        Ok(())
    }

    /// A free id like `nord-custom`, `nord-custom-2` for a new theme.
    pub fn free_id(&self, base: &str) -> ThemeId {
        let slug: String = base
            .chars()
            .map(|c| {
                if c.is_ascii_alphanumeric() {
                    c.to_ascii_lowercase()
                } else {
                    '-'
                }
            })
            .collect();
        let slug = slug.trim_matches('-').to_owned();
        let slug = if slug.is_empty() {
            "theme".to_owned()
        } else {
            slug
        };
        (1..)
            .map(|n| {
                if n == 1 {
                    slug.clone()
                } else {
                    format!("{slug}-{n}")
                }
            })
            .map(ThemeId::new)
            .find(|id| !self.themes.contains_key(id) && !Self::is_preset(id))
            .unwrap_or_else(|| ThemeId::new(slug))
    }

    /// Loads every theme file in `dir`, in name order so a file can extend one
    /// loaded before it. A broken file is reported and skipped, never fatal.
    pub fn load_dir(&mut self, dir: &Path) -> Vec<ThemeLoadError> {
        let mut paths = match theme_files(dir) {
            Ok(paths) => paths,
            Err(e) if e.kind() == std::io::ErrorKind::NotFound => return Vec::new(),
            Err(e) => {
                return vec![ThemeLoadError {
                    path: dir.to_owned(),
                    error: e.into(),
                }];
            }
        };
        paths.sort();
        paths
            .into_iter()
            .filter_map(|path| {
                self.load_file(&path)
                    .err()
                    .map(|error| ThemeLoadError { path, error })
            })
            .collect()
    }

    pub fn load_file(&mut self, path: &Path) -> Result<ThemeId, ThemeError> {
        let stem = path
            .file_stem()
            .and_then(|s| s.to_str())
            .filter(|s| !s.is_empty());
        let id = ThemeId::new(stem.ok_or(ThemeError::Invalid("theme file needs a UTF-8 name"))?);
        if presets::all().iter().any(|(preset, _)| *preset == id) {
            return Err(ThemeError::ReservedId(id));
        }
        let text = fs::read_to_string(path)?;
        let raw = parse_raw(path, &text)?;
        let tokens = self.resolve(raw)?;
        self.themes.insert(id.clone(), tokens);
        Ok(id)
    }

    /// Merges a raw theme over its `extends` base and validates the result.
    pub fn resolve(&self, mut raw: Value) -> Result<ThemeTokens, ThemeError> {
        let base = take_extends(&mut raw)?;
        let merged = match base {
            Some(id) => {
                let base = self
                    .get(&id)
                    .ok_or_else(|| ThemeError::UnknownBase(id.0.clone()))?;
                let mut merged = serde_json::to_value(base)?;
                merge(&mut merged, raw);
                merged
            }
            None => raw,
        };
        let tokens: ThemeTokens = serde_json::from_value(merged)?;
        tokens.validate()?;
        Ok(tokens)
    }
}

fn theme_files(dir: &Path) -> std::io::Result<Vec<PathBuf>> {
    let mut out = Vec::new();
    for entry in fs::read_dir(dir)? {
        let path = entry?.path();
        if matches!(extension(&path), Some("toml" | "json")) {
            out.push(path);
        }
    }
    Ok(out)
}

fn extension(path: &Path) -> Option<&str> {
    path.extension().and_then(|e| e.to_str())
}

fn parse_raw(path: &Path, text: &str) -> Result<Value, ThemeError> {
    match extension(path) {
        Some("toml") => Ok(toml::from_str(text)?),
        Some("json") => Ok(serde_json::from_str(text)?),
        _ => Err(ThemeError::UnsupportedFormat),
    }
}

fn take_extends(raw: &mut Value) -> Result<Option<ThemeId>, ThemeError> {
    let Some(table) = raw.as_object_mut() else {
        return Err(ThemeError::Invalid(
            "a theme file must be a table of tokens",
        ));
    };
    match table.remove("extends") {
        None => Ok(None),
        Some(Value::String(id)) => Ok(Some(ThemeId::new(id))),
        Some(_) => Err(ThemeError::Invalid("`extends` must be a theme id string")),
    }
}

/// Deep merge: tables merge key by key, anything else replaces.
fn merge(base: &mut Value, overlay: Value) {
    match (base, overlay) {
        (Value::Object(base), Value::Object(overlay)) => {
            for (key, value) in overlay {
                match base.get_mut(&key) {
                    Some(slot) => merge(slot, value),
                    None => {
                        base.insert(key, value);
                    }
                }
            }
        }
        (slot, value) => *slot = value,
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn write(dir: &Path, name: &str, text: &str) -> std::io::Result<()> {
        fs::write(dir.join(name), text)
    }

    #[test]
    fn toml_extends_preset_and_overrides_a_few_tokens() -> Result<(), Box<dyn std::error::Error>> {
        let dir = tempfile::tempdir()?;
        let text = "extends = \"nord\"\nname = \"Frost\"\n[colors]\naccent = \"#8fbcbb\"\n[glass]\nopacity = 0.6\n";
        write(dir.path(), "frost.toml", text)?;

        let mut themes = ThemeRegistry::with_presets();
        assert!(themes.load_dir(dir.path()).is_empty());

        let frost = themes
            .get(&ThemeId::new("frost"))
            .ok_or("frost not loaded")?;
        let nord = presets::nord();
        assert_eq!(frost.name, "Frost");
        assert_eq!(frost.colors.accent, Rgba::hex(0x8fbcbb));
        assert_eq!(frost.glass.opacity, 0.6);
        assert_eq!(frost.colors.text, nord.colors.text);
        Ok(())
    }

    #[test]
    fn json_can_extend_an_earlier_custom_theme() -> Result<(), Box<dyn std::error::Error>> {
        let dir = tempfile::tempdir()?;
        write(
            dir.path(),
            "a.toml",
            "extends = \"tokyo-night\"\nname = \"A\"\n",
        )?;
        write(
            dir.path(),
            "b.json",
            r#"{"extends": "a", "shape": {"radius_md": 2.0}}"#,
        )?;

        let mut themes = ThemeRegistry::with_presets();
        assert!(themes.load_dir(dir.path()).is_empty());
        let b = themes.get(&ThemeId::new("b")).ok_or("b not loaded")?;
        assert_eq!(b.name, "A");
        assert_eq!(b.shape.radius_md, 2.0);
        Ok(())
    }

    #[test]
    fn bad_files_are_reported_not_fatal() -> Result<(), Box<dyn std::error::Error>> {
        let dir = tempfile::tempdir()?;
        write(
            dir.path(),
            "typo.toml",
            "extends = \"nord\"\n[colors]\naccnet = \"#ffffff\"\n",
        )?;
        write(dir.path(), "nord.toml", "extends = \"nord\"\n")?;
        write(dir.path(), "ghost.toml", "extends = \"missing\"\n")?;
        write(
            dir.path(),
            "loud.toml",
            "extends = \"nord\"\n[glass]\nopacity = 4.0\n",
        )?;
        write(dir.path(), "notes.txt", "ignored")?;

        let mut themes = ThemeRegistry::with_presets();
        let errors = themes.load_dir(dir.path());
        assert_eq!(errors.len(), 4);
        assert_eq!(themes.ids().count(), presets::all().len());
        Ok(())
    }

    #[test]
    fn custom_themes_save_reload_and_delete() -> Result<(), Box<dyn std::error::Error>> {
        let dir = tempfile::tempdir()?;
        let mut themes = ThemeRegistry::with_presets();
        let id = themes.free_id("Nord Custom");
        assert_eq!(id.as_str(), "nord-custom");
        let mut tokens = presets::nord();
        tokens.name = "Nord Custom".into();
        tokens.colors.accent = Rgba::hex(0xbf616a);
        themes.save_custom(dir.path(), &id, tokens.clone())?;
        assert_eq!(themes.free_id("Nord Custom").as_str(), "nord-custom-2");

        let mut fresh = ThemeRegistry::with_presets();
        assert!(fresh.load_dir(dir.path()).is_empty());
        assert_eq!(fresh.get(&id), Some(&tokens));

        assert!(
            themes
                .save_custom(dir.path(), &ThemeId::new("nord"), tokens)
                .is_err()
        );
        themes.delete_custom(dir.path(), &id)?;
        assert!(themes.get(&id).is_none());
        assert!(!dir.path().join("nord-custom.toml").exists());
        Ok(())
    }

    #[test]
    fn missing_dir_is_fine() {
        let mut themes = ThemeRegistry::with_presets();
        assert!(themes.load_dir(Path::new("does/not/exist")).is_empty());
    }
}
