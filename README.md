<p align="center">
  <img src="docs/images/umbrella-icon.png" alt="Umbrella icon" width="160">
</p>

# Umbrella

A small Windows browser written in Rust. The tab strip, address bar and
settings are drawn natively with [GPUI](https://crates.io/crates/gpui-ce);
web pages are rendered by Microsoft's WebView2, the same engine Edge uses.

It is a personal project and still early. It works well enough to use every
day, but expect rough edges.

This was also vibe slopped for fun purely to have my own browser I can work on and have something which won't die on me :D

## What it does

- Restores your tabs on launch without loading them. Only the tabs you can
  see get a page process; tabs you have not looked at for a while are put to
  sleep and wake up when you return to them.
- Blocks ads and trackers with [adblock-rust](https://github.com/brave/adblock-rust)
  using EasyList and EasyPrivacy, and strips `utm_`, `fbclid` and similar
  parameters from links.
- Loads unpacked Chrome extensions. uBlock Origin Lite works and handles
  sites the built-in blocker cannot, YouTube included.
- Keeps passwords in its own encrypted vault (XChaCha20-Poly1305, with the
  key derived by Argon2id or held by Windows DPAPI). Saved logins only ever
  fill on the exact site they were saved for. On sign-up forms it offers a
  generated password under the field; length and character sets are in
  Settings > Passwords.
- Has workspaces (separate sets of tabs with separate cookies), split view,
  a command palette, history search, find in page, reader view, and a page
  info panel showing the response headers and which other sites saw your IP.
- Themes: Safari Glass Dark, Catppuccin Mocha, Tokyo Night and Nord, plus
  your own. Any theme can be copied and recoloured from Settings.

## Building

You need Windows 10 or 11 with the WebView2 runtime (already present on
Windows 11) and a recent stable Rust toolchain.

```
cargo build --release -p browser-ui
target\release\umbrella.exe
```

A debug build (`cargo build -p browser-ui`) is much faster to compile and
fine for trying things out.

Passing URLs opens them as tabs: `umbrella.exe https://example.com`. If
Umbrella is already running, the URLs are handed to the open window instead
of starting a second one.

## Where your data lives

Everything is in `%LOCALAPPDATA%\Umbrella`:

| Path | What |
| --- | --- |
| `profile.redb` | Tabs, history, bookmarks, settings, site permissions |
| `vault.redb` | The password vault (encrypted) |
| `webview\workspace-N\` | Cookies, cache and site storage, one folder per workspace |
| `themes\` | Your custom themes, as TOML |
| `extensions\` | Extensions you added from a .zip |
| `scripts\` | Userscripts (`*.user.js`) and user styles (`*.css`) |

Set `UMBRELLA_HOME` to use a different folder, which is handy for testing
without touching your real profile.

Nothing is sent anywhere by Umbrella itself. The only network requests it
makes on its own are fetching the two filter lists, at most every four days.

## Keyboard

| Keys | Action |
| --- | --- |
| Ctrl+T / Ctrl+W | New tab / close tab |
| Ctrl+Shift+T | Reopen the last closed tab |
| Ctrl+Tab | Switch to the previously used tab |
| Ctrl+L or Alt+D | Address bar |
| Ctrl+K or F2 | Command palette (`>` in the address bar does the same) |
| Ctrl+F | Find in page |
| Ctrl+H | History (or type `#` in the address bar) |
| Ctrl+D | Bookmark the page |
| Ctrl+, | Settings |
| Ctrl+Plus / Minus / 0 | Zoom |
| Ctrl+Shift+PgUp / PgDn | Move the tab left or right |
| Alt+Left / Alt+Right | Back / forward |

## Custom themes

Copy a theme from Settings > Appearance and change its colours there, or
drop a file into the `themes` folder. A file only needs the tokens it
changes:

```toml
extends = "nord"
name = "Frost"

[colors]
accent = "#8fbcbb"

[glass]
opacity = 0.75
```

The file name (`frost.toml`) becomes the theme's id. Settings warns you when
a colour makes text hard to read.

## How it is put together

| Crate | Job |
| --- | --- |
| `browser-core` | Sessions, tabs, history, settings, themes, springs, the vault |
| `browser-net` | Ad blocking, filter list updates, favicons, userscripts |
| `browser-engine` | Everything that talks to WebView2 |
| `browser-ui` | The window, the chrome and the `umbrella` binary |

`browser-core` and `browser-net` have no Windows dependencies and carry most
of the tests (`cargo test --workspace`). The reasoning behind a few of the
larger choices is written up in `docs/decisions/`, and `DESIGN.md` describes
the look of the chrome.

## Known limits

- WebView2 cannot be drawn over, so menus and prompts that would normally
  float over a page sit in the toolbar instead, or hide the page while they
  are open. The generated-password offer is the exception: the page draws
  it under the field, so the page can also see or hide it.
- Extensions have no toolbar buttons; their settings and popups open as
  tabs from Settings > Extensions.
- Task Manager lists the page processes as "WebView2". That label comes
  from Microsoft's runtime and cannot be changed.
- Windows only.
