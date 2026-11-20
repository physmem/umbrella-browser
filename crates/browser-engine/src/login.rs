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
    /// A new-password field took focus; the page may show an offer.
    SuggestWanted,
    /// The user accepted the offer; generate and fill.
    SuggestAccepted,
}

/// Colours for the in-page offer, as CSS colour strings from the theme.
pub struct OfferTheme {
    pub surface: String,
    pub text: String,
    pub muted: String,
    pub border: String,
    pub hover: String,
    pub selection: String,
}

/// The browser's answer to `SuggestWanted`: show the offer, in these colours.
pub fn offer_message(theme: &OfferTheme) -> String {
    serde_json::json!({
        "umbrella": "login",
        "kind": "offer",
        "theme": {
            "surface": theme.surface,
            "text": theme.text,
            "muted": theme.muted,
            "border": theme.border,
            "hover": theme.hover,
            "selection": theme.selection,
        },
    })
    .to_string()
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
        "suggest" => Some(LoginMessage::SuggestWanted),
        "suggest-accept" => Some(LoginMessage::SuggestAccepted),
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

/// Fills a generated password into the focused new-password field and the
/// confirm field beside it. Like `fill_script`, it re-checks the origin.
pub fn generated_fill_script(origin: &Origin, password: &str) -> Zeroizing<String> {
    let mut script = Zeroizing::new(String::with_capacity(GENERATED_TEMPLATE.len() + 256));
    script.push_str("(() => {\nconst want = ");
    push_json(&mut script, &origin.canonical());
    script.push_str(";\nconst pass = ");
    push_json(&mut script, password);
    script.push_str(";\n");
    script.push_str(GENERATED_TEMPLATE);
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

/// The target is the focused field: the offer keeps focus there. Its confirm
/// field is the next password field in the form, or the one before it when
/// the user started on the confirm field and the first is still empty.
const GENERATED_TEMPLATE: &str = r#"const l = location;
const port = l.port || (l.protocol === "https:" ? "443" : "80");
if (`${l.protocol}//${l.hostname}:${port}` !== want) return false;
const pw = document.activeElement;
if (!(pw instanceof HTMLInputElement) || pw.type !== "password") return false;
const setter = Object.getOwnPropertyDescriptor(HTMLInputElement.prototype, "value").set;
const set = (el, v) => {
  setter.call(el, v);
  el.dispatchEvent(new Event("input", { bubbles: true }));
  el.dispatchEvent(new Event("change", { bubbles: true }));
};
const all = [...(pw.form || document).querySelectorAll("input[type=password]")];
const i = all.indexOf(pw);
set(pw, pass);
if (all[i + 1]) set(all[i + 1], pass);
else if (all.length === 2 && i === 1 && !all[0].value) set(all[0], pass);
pw.dispatchEvent(new CustomEvent("umbrella-generated", { bubbles: true, composed: true }));
return true;
})()"#;

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn parses_suggest_messages() {
        assert!(matches!(
            parse_message(r#"{"umbrella":"login","kind":"suggest"}"#),
            Some(LoginMessage::SuggestWanted)
        ));
        assert!(matches!(
            parse_message(r#"{"umbrella":"login","kind":"suggest-accept"}"#),
            Some(LoginMessage::SuggestAccepted)
        ));
    }

    #[test]
    fn generated_fill_checks_origin_and_escapes() -> Result<(), browser_core::vault::OriginError> {
        let origin = Origin::parse("https://example.com/signup")?;
        let script = generated_fill_script(&origin, "a\"b</script>");
        assert!(script.contains(r#"const want = "https://example.com:443";"#));
        assert!(script.contains(r#"const pass = "a\"b</script>";"#));
        assert!(script.contains("!== want) return false;"));
        Ok(())
    }

    #[test]
    fn offer_message_is_tagged_json() -> Result<(), serde_json::Error> {
        let theme = OfferTheme {
            surface: "#3a3a3c".into(),
            text: "#f5f5f7".into(),
            muted: "#aeaeb2".into(),
            border: "#ffffff1f".into(),
            hover: "#f5f5f712".into(),
            selection: "#0071e338".into(),
        };
        let value: serde_json::Value = serde_json::from_str(&offer_message(&theme))?;
        assert_eq!(value["umbrella"], "login");
        assert_eq!(value["kind"], "offer");
        assert_eq!(value["theme"]["surface"], "#3a3a3c");
        Ok(())
    }

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
