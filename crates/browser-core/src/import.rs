//! Bringing data over from other browsers: bookmarks straight from a
//! Chromium profile's `Bookmarks` file, passwords from the CSV that
//! Chrome, Edge and Brave export. Other browsers' encrypted password stores
//! are never read directly.

use std::path::{Path, PathBuf};

use serde_json::Value;
use zeroize::Zeroizing;

/// A Chromium browser whose default profile may hold bookmarks.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct SourceBrowser {
    pub name: &'static str,
    pub bookmarks: PathBuf,
}

const CHROMIUM_PROFILES: &[(&str, &str)] = &[
    ("Chrome", r"Google\Chrome\User Data\Default\Bookmarks"),
    ("Edge", r"Microsoft\Edge\User Data\Default\Bookmarks"),
    (
        "Brave",
        r"BraveSoftware\Brave-Browser\User Data\Default\Bookmarks",
    ),
    ("Vivaldi", r"Vivaldi\User Data\Default\Bookmarks"),
];

/// Browsers under `local_app_data` that have a bookmarks file.
pub fn installed_browsers(local_app_data: &Path) -> Vec<SourceBrowser> {
    CHROMIUM_PROFILES
        .iter()
        .map(|(name, rel)| SourceBrowser {
            name,
            bookmarks: local_app_data.join(rel),
        })
        .filter(|b| b.bookmarks.is_file())
        .collect()
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct ImportedBookmark {
    pub title: String,
    pub url: String,
}

/// Every web bookmark in a Chromium `Bookmarks` JSON file, folders flattened.
pub fn chromium_bookmarks(json: &str) -> Vec<ImportedBookmark> {
    let Ok(root) = serde_json::from_str::<Value>(json) else {
        return Vec::new();
    };
    let mut out = Vec::new();
    if let Some(roots) = root.get("roots").and_then(Value::as_object) {
        for node in roots.values() {
            collect(node, &mut out, 0);
        }
    }
    out
}

fn collect(node: &Value, out: &mut Vec<ImportedBookmark>, depth: usize) {
    // Real folders nest a few levels; a cap keeps a crafted file harmless.
    if depth > 64 {
        return;
    }
    match node.get("type").and_then(Value::as_str) {
        Some("url") => {
            let url = node.get("url").and_then(Value::as_str).unwrap_or_default();
            if url.starts_with("http://") || url.starts_with("https://") {
                out.push(ImportedBookmark {
                    title: node
                        .get("name")
                        .and_then(Value::as_str)
                        .unwrap_or_default()
                        .to_owned(),
                    url: url.to_owned(),
                });
            }
        }
        _ => {
            for child in node
                .get("children")
                .and_then(Value::as_array)
                .into_iter()
                .flatten()
            {
                collect(child, out, depth + 1);
            }
        }
    }
}

/// One login from a password CSV. Fields are wiped from memory on drop.
pub struct ImportedLogin {
    pub url: String,
    pub username: Zeroizing<String>,
    pub password: Zeroizing<String>,
}

/// Parses the `name,url,username,password[,note]` CSV the Chromium
/// browsers export. Columns are found by header name, so order may vary.
pub fn password_csv(text: &str) -> Result<Vec<ImportedLogin>, String> {
    let mut rows = csv_rows(text).into_iter();
    let header = rows.next().ok_or("the file is empty")?;
    let column = |name: &str| {
        header
            .iter()
            .position(|h| h.trim().eq_ignore_ascii_case(name))
            .ok_or(format!("the file has no '{name}' column"))
    };
    let (url, user, pass) = (column("url")?, column("username")?, column("password")?);
    Ok(rows
        .filter_map(|row| {
            let login = ImportedLogin {
                url: row.get(url)?.trim().to_owned(),
                username: Zeroizing::new(row.get(user)?.clone()),
                password: Zeroizing::new(row.get(pass)?.clone()),
            };
            (!login.url.is_empty() && !login.password.is_empty()).then_some(login)
        })
        .collect())
}

/// RFC 4180 rows: quoted fields may hold commas, quotes ("") and newlines.
fn csv_rows(text: &str) -> Vec<Vec<String>> {
    let mut rows = Vec::new();
    let (mut row, mut field) = (Vec::new(), String::new());
    let mut quoted = false;
    let mut chars = text.trim_start_matches('\u{feff}').chars().peekable();
    while let Some(c) = chars.next() {
        match (c, quoted) {
            ('"', true) if chars.peek() == Some(&'"') => {
                field.push('"');
                chars.next();
            }
            ('"', true) => quoted = false,
            ('"', false) if field.is_empty() => quoted = true,
            (',', false) => row.push(std::mem::take(&mut field)),
            ('\r', false) => {}
            ('\n', false) => {
                row.push(std::mem::take(&mut field));
                rows.push(std::mem::take(&mut row));
            }
            (c, _) => field.push(c),
        }
    }
    if !field.is_empty() || !row.is_empty() {
        row.push(field);
        rows.push(row);
    }
    rows
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn flattens_chromium_bookmark_folders() {
        let json = r#"{"roots":{
            "bookmark_bar":{"type":"folder","children":[
                {"type":"url","name":"Rust","url":"https://www.rust-lang.org/"},
                {"type":"folder","name":"Work","children":[
                    {"type":"url","name":"Docs","url":"https://docs.rs/"},
                    {"type":"url","name":"Script","url":"javascript:alert(1)"}
                ]}
            ]},
            "other":{"type":"folder","children":[]}
        }}"#;
        let marks = chromium_bookmarks(json);
        assert_eq!(marks.len(), 2);
        assert_eq!(marks[1].title, "Docs");
        assert!(chromium_bookmarks("not json").is_empty());
    }

    #[test]
    fn parses_exported_password_csv() -> Result<(), String> {
        let csv = "name,url,username,password,note\r\n\
                   example.com,https://example.com/login,ann,\"p,a\"\"ss\",\r\n\
                   empty,https://empty.example/,bob,,\r\n";
        let logins = password_csv(csv)?;
        assert_eq!(logins.len(), 1);
        assert_eq!(logins[0].url, "https://example.com/login");
        assert_eq!(*logins[0].username, "ann");
        assert_eq!(*logins[0].password, "p,a\"ss");
        assert!(password_csv("name,user\n").is_err());
        Ok(())
    }
}
