//! Login form detection and fill. Messages from pages are untrusted: any
//! page can call `postMessage`, so the origin always comes from WebView2's
//! own record of the sender, never from the message body.

use browser_core::vault::{Credential, Origin};
use serde::Deserialize;
use zeroize::Zeroizing;

pub const LOGIN_BRIDGE_JS: &str = include_str!("login_bridge.js");

#[derive(Debug)]
pub enum LoginMessage {
    FormSeen,
    Submitted(Credential),
}

#[derive(Deserialize)]
struct Wire {
    umbrella: String,
    kind: String,
    #[serde(default)]
    username: String,
    #[serde(default)]
    password: String,
}

/// Parses a `WebMessageAsJson` payload. Unrelated page messages yield `None`.
pub fn parse_message(json: &str) -> Option<LoginMessage> {
    let wire: Wire = serde_json::from_str(json).ok()?;
    if wire.umbrella != "login" {
        return None;
    }
    match wire.kind.as_str() {
        "form" => Some(LoginMessage::FormSeen),
        "submit" if !wire.password.is_empty() => Some(LoginMessage::Submitted(Credential {
            // Moved, not copied, so only the zeroizing owner holds the text.
            username: Zeroizing::new(wire.username),
            password: Zeroizing::new(wire.password),
        })),
        _ => None,
    }
}

/// A self-contained fill script. It re-checks the live document's origin so
/// a navigation between the prompt and the click cannot redirect the fill.
pub fn fill_script(origin: &Origin, username: &str, password: &str) -> Zeroizing<String> {
    let mut script = Zeroizing::new(String::with_capacity(FILL_TEMPLATE.len() + 256));
    script.push_str("(() => {\nconst want = ");
    push_json(&mut script, &origin.canonical());
    script.push_str(";\nconst user = ");
    push_json(&mut script, username);
    script.push_str(";\nconst pass = ");
    push_json(&mut script, password);
    script.push_str(";\n");
    script.push_str(FILL_TEMPLATE);
    script
}

/// Writes a JSON string literal (also a valid JS literal) without an
/// intermediate allocation that would escape zeroization.
fn push_json(out: &mut String, text: &str) {
    out.push('"');
    for c in text.chars() {
        match c {
            '"' => out.push_str("\\\""),
            '\\' => out.push_str("\\\\"),
            '\n' => out.push_str("\\n"),
            '\r' => out.push_str("\\r"),
            '\t' => out.push_str("\\t"),
            c if (c as u32) < 0x20 || c == '\u{2028}' || c == '\u{2029}' => {
                out.push_str(&format!("\\u{:04x}", c as u32));
            }
            c => out.push(c),
        }
    }
    out.push('"');
}

const FILL_TEMPLATE: &str = r#"const l = location;
const port = l.port || (l.protocol === "https:" ? "443" : "80");
if (`${l.protocol}//${l.hostname}:${port}` !== want) return false;
const pw = document.querySelector("input[type=password]");
if (!pw) return false;
const setter = Object.getOwnPropertyDescriptor(HTMLInputElement.prototype, "value").set;
const set = (el, v) => {
  setter.call(el, v);
  el.dispatchEvent(new Event("input", { bubbles: true }));
  el.dispatchEvent(new Event("change", { bubbles: true }));
};
const scope = pw.form || document;
const textLike = "input:not([type=hidden]):not([type=password]):not([type=checkbox])" +
  ":not([type=radio]):not([type=submit]):not([type=button])";
const field = [...scope.querySelectorAll(textLike)]
  .filter((i) => pw.compareDocumentPosition(i) & Node.DOCUMENT_POSITION_PRECEDING)
  .pop();
if (field && user) set(field, user);
set(pw, pass);
return true;
})()"#;

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn parses_form_and_submit() {
        assert!(matches!(
            parse_message(r#"{"umbrella":"login","kind":"form"}"#),
            Some(LoginMessage::FormSeen)
        ));
        let json = r#"{"umbrella":"login","kind":"submit","username":"ada","password":"pw"}"#;
        let Some(LoginMessage::Submitted(cred)) = parse_message(json) else {
            panic!("expected submit");
        };
        assert_eq!(cred.username.as_str(), "ada");
        assert_eq!(cred.password.as_str(), "pw");
    }

    #[test]
    fn ignores_foreign_and_empty_messages() {
        assert!(parse_message(r#"{"type":"analytics"}"#).is_none());
        assert!(parse_message(r#"{"umbrella":"other","kind":"form"}"#).is_none());
        assert!(parse_message(r#"{"umbrella":"login","kind":"submit","password":""}"#).is_none());
        assert!(parse_message("not json").is_none());
    }

    #[test]
    fn fill_script_escapes_and_checks_origin() -> Result<(), browser_core::vault::OriginError> {
        let origin = Origin::parse("https://example.com/login")?;
        let script = fill_script(&origin, "ada", "a\"b\\c\u{2028}</script>");
        assert!(script.contains(r#"const want = "https://example.com:443";"#));
        // Built from escapes so no raw U+2028 sits in this source file.
        let expected = concat!("const pass = \"a\\\"b\\\\c", "\\", "u2028</script>\";");
        assert!(script.contains(expected));
        assert!(script.contains("!== want) return false;"));
        Ok(())
    }

    #[test]
    fn json_escaping_matches_serde() {
        for text in [
            "plain",
            "quote\"back\\slash",
            "tab\tnew\nline",
            "\u{1}ctl",
            "emoji 🔑",
        ] {
            let mut ours = String::new();
            push_json(&mut ours, text);
            let parsed: String = serde_json::from_str(&ours).unwrap_or_default();
            assert_eq!(parsed, text);
        }
    }
}
