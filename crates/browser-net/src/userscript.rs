//! `.user.js` scripts and `.css` styles from a local folder, compiled into
//! self-guarding bundles. WebView2 can only inject a script into every
//! document, so each bundle checks the URL itself before running.

use std::fs;
use std::path::{Path, PathBuf};

use regex::Regex;
use thiserror::Error;

#[derive(Debug, Error)]
pub enum ScriptError {
    #[error("cannot read script: {0}")]
    Io(#[from] std::io::Error),
    #[error("missing ==UserScript== header")]
    MissingHeader,
    #[error("bad URL pattern `{pattern}`: {source}")]
    BadPattern {
        pattern: String,
        source: regex::Error,
    },
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum RunAt {
    DocumentStart,
    DocumentEnd,
    DocumentIdle,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum ScriptKind {
    Script,
    Style,
}

#[derive(Debug, Clone)]
pub struct UserScript {
    pub name: String,
    pub kind: ScriptKind,
    pub run_at: RunAt,
    pub path: PathBuf,
    source: String,
    include: Vec<Regex>,
    exclude: Vec<Regex>,
}

#[derive(Debug, Default)]
struct Metadata {
    name: Option<String>,
    run_at: Option<RunAt>,
    include: Vec<String>,
    exclude: Vec<String>,
    found: bool,
}

impl UserScript {
    pub fn parse_script(path: &Path, source: &str) -> Result<Self, ScriptError> {
        let meta = parse_metadata(source, "==UserScript==", "==/UserScript==");
        if !meta.found {
            return Err(ScriptError::MissingHeader);
        }
        Self::build(path, source, meta, ScriptKind::Script, RunAt::DocumentEnd)
    }

    /// The header is optional for styles; without one they apply everywhere.
    pub fn parse_style(path: &Path, source: &str) -> Result<Self, ScriptError> {
        let meta = parse_metadata(source, "==UserStyle==", "==/UserStyle==");
        Self::build(path, source, meta, ScriptKind::Style, RunAt::DocumentStart)
    }

    fn build(
        path: &Path,
        source: &str,
        meta: Metadata,
        kind: ScriptKind,
        default_run_at: RunAt,
    ) -> Result<Self, ScriptError> {
        let mut include = meta.include;
        if include.is_empty() {
            include.push(ALL_URLS.to_owned());
        }
        let script = Self {
            name: meta.name.unwrap_or_else(|| file_label(path)),
            kind,
            run_at: meta.run_at.unwrap_or(default_run_at),
            path: path.to_owned(),
            source: source.to_owned(),
            include: compile_all(&include)?,
            exclude: compile_all(&meta.exclude)?,
        };
        Ok(script)
    }

    pub fn matches(&self, url: &str) -> bool {
        let hit = |patterns: &[Regex]| patterns.iter().any(|r| r.is_match(url));
        hit(&self.include) && !hit(&self.exclude)
    }

    /// JavaScript to register with `AddScriptToExecuteOnDocumentCreated`.
    pub fn bundle(&self) -> String {
        let body = match self.kind {
            ScriptKind::Script => self.source.clone(),
            ScriptKind::Style => style_body(&self.source),
        };
        format!(
            "(() => {{\nconst inc = [{inc}];\nconst exc = [{exc}];\nconst href = location.href;\n\
             if (!inc.some(r => r.test(href)) || exc.some(r => r.test(href))) return;\n\
             const run = () => {{ try {{\n{body}\n}} catch (e) {{ console.error({label}, e); }} }};\n\
             {schedule}\n}})();",
            inc = regex_list(&self.include),
            exc = regex_list(&self.exclude),
            label = js_string(&format!("[userscript {}]", self.name)),
            schedule = schedule(self.run_at),
        )
    }
}

const ALL_URLS: &str = "^.*$";

fn parse_metadata(source: &str, open: &str, close: &str) -> Metadata {
    let mut meta = Metadata::default();
    let mut inside = false;
    for line in source.lines() {
        let line = line
            .trim()
            .trim_start_matches("/*")
            .trim_start_matches("//")
            .trim();
        if line.starts_with(open) {
            inside = true;
            meta.found = true;
            continue;
        }
        if line.starts_with(close) {
            break;
        }
        if inside {
            apply_meta_line(&mut meta, line);
        }
    }
    meta
}

fn apply_meta_line(meta: &mut Metadata, line: &str) {
    let Some((key, value)) = line
        .strip_prefix('@')
        .and_then(|l| l.split_once(char::is_whitespace))
    else {
        return;
    };
    let value = value.trim().to_owned();
    match key {
        "name" => meta.name = Some(value),
        "match" => meta.include.push(match_pattern_to_regex(&value)),
        "include" => meta.include.push(glob_to_regex(&value)),
        "exclude" | "exclude-match" => meta.exclude.push(glob_to_regex(&value)),
        "run-at" => meta.run_at = parse_run_at(&value).or(meta.run_at),
        _ => {}
    }
}

fn parse_run_at(value: &str) -> Option<RunAt> {
    match value {
        "document-start" => Some(RunAt::DocumentStart),
        "document-end" => Some(RunAt::DocumentEnd),
        "document-idle" => Some(RunAt::DocumentIdle),
        _ => None,
    }
}

/// Chrome match pattern: `<scheme>://<host><path>`, where `*.host` also
/// matches the bare host and an omitted port matches any port.
fn match_pattern_to_regex(pattern: &str) -> String {
    if pattern == "<all_urls>" {
        return ALL_URLS.to_owned();
    }
    let Some((scheme, rest)) = pattern.split_once("://") else {
        return glob_to_regex(pattern);
    };
    let (host, path) = rest.split_at(rest.find('/').unwrap_or(rest.len()));
    let scheme = if scheme == "*" {
        "https?".to_owned()
    } else {
        regex::escape(scheme)
    };
    let host = match host {
        "*" => "[^/]*".to_owned(),
        h if h.starts_with("*.") => format!("(?:[^/]*\\.)?{}(?::\\d+)?", regex::escape(&h[2..])),
        h => format!("{}(?::\\d+)?", regex::escape(h)),
    };
    let path = if path.is_empty() {
        ".*".to_owned()
    } else {
        glob_body(path)
    };
    format!("^{scheme}://{host}{path}$")
}

/// Greasemonkey `@include`: a glob over the whole URL, or `/regex/`.
fn glob_to_regex(glob: &str) -> String {
    if let Some(raw) = glob.strip_prefix('/').and_then(|g| g.strip_suffix('/')) {
        return raw.to_owned();
    }
    format!("^{}$", glob_body(glob))
}

fn glob_body(glob: &str) -> String {
    glob.split('*')
        .map(regex::escape)
        .collect::<Vec<_>>()
        .join(".*")
}

fn compile_all(patterns: &[String]) -> Result<Vec<Regex>, ScriptError> {
    patterns
        .iter()
        .map(|p| {
            Regex::new(p).map_err(|source| ScriptError::BadPattern {
                pattern: p.clone(),
                source,
            })
        })
        .collect()
}

// Emitted patterns stick to syntax shared by Rust and JS regex engines.
fn regex_list(patterns: &[Regex]) -> String {
    patterns
        .iter()
        .map(|r| format!("new RegExp({})", js_string(r.as_str())))
        .collect::<Vec<_>>()
        .join(", ")
}

fn schedule(run_at: RunAt) -> &'static str {
    match run_at {
        RunAt::DocumentStart => "run();",
        RunAt::DocumentEnd => {
            "if (document.readyState === 'loading') document.addEventListener('DOMContentLoaded', run, { once: true }); else run();"
        }
        RunAt::DocumentIdle => {
            "if (document.readyState === 'complete') setTimeout(run); else addEventListener('load', () => setTimeout(run), { once: true });"
        }
    }
}

// At document start there may be no <html> yet, so wait for a root.
fn style_body(css: &str) -> String {
    format!(
        "const s = document.createElement('style'); s.textContent = {css};\n\
         const add = () => {{ const root = document.head || document.documentElement; if (!root) return false; root.appendChild(s); return true; }};\n\
         if (!add()) new MutationObserver((_, o) => {{ if (add()) o.disconnect(); }}).observe(document, {{ childList: true, subtree: true }});",
        css = js_string(css),
    )
}

/// JSON strings are valid JS string literals, including U+2028/2029.
fn js_string(text: &str) -> String {
    serde_json::to_string(text).unwrap_or_else(|_| "\"\"".to_owned())
}

fn file_label(path: &Path) -> String {
    path.file_name()
        .map(|n| n.to_string_lossy().into_owned())
        .unwrap_or_default()
}

/// Every script and style in a folder, plus the files that failed to load.
#[derive(Debug, Default)]
pub struct ScriptLibrary {
    pub scripts: Vec<UserScript>,
    pub errors: Vec<(PathBuf, ScriptError)>,
}

impl ScriptLibrary {
    pub fn load_dir(dir: &Path) -> std::io::Result<Self> {
        let mut library = Self::default();
        let entries = match fs::read_dir(dir) {
            Ok(entries) => entries,
            Err(e) if e.kind() == std::io::ErrorKind::NotFound => return Ok(library),
            Err(e) => return Err(e),
        };
        let mut paths: Vec<PathBuf> = entries.filter_map(|e| e.ok().map(|e| e.path())).collect();
        paths.sort();
        for path in paths {
            library.load_file(path);
        }
        Ok(library)
    }

    pub fn bundles(&self) -> Vec<String> {
        self.scripts.iter().map(UserScript::bundle).collect()
    }

    fn load_file(&mut self, path: PathBuf) {
        let name = file_label(&path);
        let parse = if name.ends_with(".user.js") {
            UserScript::parse_script
        } else if name.ends_with(".css") {
            UserScript::parse_style
        } else {
            return;
        };
        let result = fs::read_to_string(&path)
            .map_err(ScriptError::from)
            .and_then(|text| parse(&path, &text));
        match result {
            Ok(script) => self.scripts.push(script),
            Err(error) => self.errors.push((path, error)),
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    const DARK_GH: &str = "// ==UserScript==\n// @name Dark GitHub\n// @match https://*.github.com/*\n\
                           // @exclude https://gist.github.com/*\n// @run-at document-start\n\
                           // ==/UserScript==\ndocument.documentElement.dataset.theme = 'dark';\n";

    fn script(text: &str) -> Result<UserScript, ScriptError> {
        UserScript::parse_script(Path::new("dark.user.js"), text)
    }

    #[test]
    fn parses_header() -> Result<(), ScriptError> {
        let s = script(DARK_GH)?;
        assert_eq!(s.name, "Dark GitHub");
        assert_eq!(s.run_at, RunAt::DocumentStart);
        Ok(())
    }

    #[test]
    fn match_pattern_semantics() -> Result<(), ScriptError> {
        let s = script(DARK_GH)?;
        assert!(s.matches("https://github.com/rust-lang"));
        assert!(s.matches("https://api.github.com/x"));
        assert!(s.matches("https://github.com:8443/x"));
        assert!(!s.matches("http://github.com/x"));
        assert!(!s.matches("https://github.com.evil.test/x"));
        assert!(!s.matches("https://gist.github.com/abc"));
        Ok(())
    }

    #[test]
    fn include_globs_and_regex() -> Result<(), ScriptError> {
        let text = "// ==UserScript==\n// @include *://*.test/*\n// @include /^https://x\\.dev/\n// ==/UserScript==\n";
        let s = script(text)?;
        assert!(s.matches("https://a.test/page"));
        assert!(s.matches("https://x.dev/anything"));
        assert!(!s.matches("https://a.example/"));
        Ok(())
    }

    #[test]
    fn no_header_is_an_error_for_scripts_only() -> Result<(), ScriptError> {
        assert!(matches!(
            script("alert(1)"),
            Err(ScriptError::MissingHeader)
        ));
        let style = UserScript::parse_style(Path::new("all.css"), "body { margin: 0 }")?;
        assert!(style.matches("https://anything.test/"));
        assert_eq!(style.name, "all.css");
        Ok(())
    }

    #[test]
    fn bad_regex_is_reported() {
        let text = "// ==UserScript==\n// @include /(unclosed/\n// ==/UserScript==\n";
        assert!(matches!(script(text), Err(ScriptError::BadPattern { .. })));
    }

    #[test]
    fn bundle_escapes_style_source() -> Result<(), ScriptError> {
        let style = UserScript::parse_style(Path::new("q.css"), "a::after { content: \"\\\"\" }")?;
        let bundle = style.bundle();
        assert!(bundle.contains(r#"s.textContent = "a::after { content: \"\\\"\" }";"#));
        Ok(())
    }

    #[test]
    fn library_loads_folder_and_collects_errors() -> std::io::Result<()> {
        let dir = tempfile::tempdir()?;
        fs::write(dir.path().join("dark.user.js"), DARK_GH)?;
        fs::write(dir.path().join("broken.user.js"), "no header")?;
        fs::write(dir.path().join("site.css"), "body{}")?;
        fs::write(dir.path().join("readme.txt"), "ignored")?;
        let library = ScriptLibrary::load_dir(dir.path())?;
        assert_eq!(library.scripts.len(), 2);
        assert_eq!(library.errors.len(), 1);
        assert_eq!(library.bundles().len(), 2);
        Ok(())
    }
}
