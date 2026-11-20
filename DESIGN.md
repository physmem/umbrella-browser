---
name: Umbrella
description: Native Windows browser chrome; one frosted slab of tabs and address over the DWM backdrop, the page owns the rest.
colors:
  light-window: "#f5f5f7"
  light-surface-raised: "#fbfbfd"
  light-text: "#1d1d1f"
  light-text-muted: "#4a4a50"
  light-accent: "#0066cc"
  light-on-accent: "#ffffff"
  light-focus: "#0071e3"
  light-border: "#0000001f"
  light-glass: "#f5f5f7db"
  light-capsule: "#fbfbfde6"
  light-capsule-edge: "#0066cc59"
  light-hover: "#1d1d1f12"
  light-field: "#1d1d1f0f"
  light-row-selected: "#0066cc38"
  dark-window: "#1c1c1e"
  dark-surface-raised: "#3a3a3c"
  dark-text: "#f5f5f7"
  dark-text-muted: "#aeaeb2"
  dark-accent: "#0071e3"
  dark-on-accent: "#ffffff"
  dark-focus: "#0a84ff"
  dark-border: "#ffffff1f"
  dark-glass: "#1c1c1eb3"
  dark-capsule: "#3a3a3ce6"
  dark-capsule-edge: "#0071e359"
  dark-hover: "#f5f5f712"
  dark-field: "#f5f5f70f"
  dark-row-selected: "#0071e338"
  caption-close: "#c42b1c"
typography:
  field:
    fontFamily: "Segoe UI Variable Text"
    fontSize: "13px"
    fontWeight: 400
  host:
    fontFamily: "Segoe UI Variable Text"
    fontSize: "13px"
    fontWeight: 500
  body:
    fontFamily: "Segoe UI Variable Text"
    fontSize: "12px"
    fontWeight: 400
  label:
    fontFamily: "Segoe UI Variable Text"
    fontSize: "11px"
    fontWeight: 400
  action:
    fontFamily: "Segoe UI Variable Text"
    fontSize: "12px"
    fontWeight: 600
  icon:
    fontFamily: "Segoe Fluent Icons"
    fontSize: "12px"
rounded:
  sm: "6px"
  md: "10px"
  lg: "14px"
  control: "6px"
  action: "5px"
  inset-field: "4px"
spacing:
  hairline: "1px"
  cluster-gap: "2px"
  status-gap: "5px"
  pill-gap: "6px"
  edge: "8px"
  split-gutter: "8px"
  row-inset: "10px"
  tab-row: "36px"
  toolbar-row: "40px"
  chrome: "76px"
components:
  tab:
    textColor: "{colors.light-text-muted}"
    typography: "{typography.body}"
    rounded: "{rounded.md}"
    padding: "0 4px 0 10px"
    height: "28px"
  tab-hover:
    backgroundColor: "{colors.light-hover}"
  tab-active-capsule:
    backgroundColor: "{colors.light-capsule}"
    textColor: "{colors.light-text}"
    rounded: "{rounded.md}"
    height: "28px"
  icon-button:
    textColor: "{colors.light-text}"
    typography: "{typography.icon}"
    rounded: "{rounded.control}"
    size: "30px"
  icon-button-hover:
    backgroundColor: "{colors.light-hover}"
  caption-button:
    typography: "{typography.icon}"
    width: "46px"
    height: "36px"
  caption-close-hover:
    backgroundColor: "{colors.caption-close}"
  omnibox:
    backgroundColor: "{colors.light-field}"
    textColor: "{colors.light-text}"
    typography: "{typography.field}"
    rounded: "{rounded.md}"
    padding: "0 10px"
    height: "28px"
  omnibox-editing:
    backgroundColor: "{colors.light-field}"
    textColor: "{colors.light-text}"
    rounded: "{rounded.md}"
  pill:
    backgroundColor: "{colors.light-capsule}"
    textColor: "{colors.light-text}"
    typography: "{typography.body}"
    rounded: "{rounded.lg}"
    padding: "0 4px 0 10px"
    height: "28px"
  action-button:
    backgroundColor: "{colors.light-accent}"
    textColor: "{colors.light-on-accent}"
    typography: "{typography.action}"
    rounded: "{rounded.action}"
    padding: "0 10px"
    height: "22px"
  status-pill:
    textColor: "{colors.light-text-muted}"
    typography: "{typography.label}"
    rounded: "{rounded.sm}"
    padding: "0 8px"
    height: "24px"
  download-pill:
    textColor: "{colors.light-text}"
    typography: "{typography.label}"
    rounded: "{rounded.sm}"
    padding: "0 8px"
    height: "24px"
  suggestion-panel:
    backgroundColor: "{colors.light-capsule}"
    typography: "{typography.field}"
    rounded: "{rounded.lg}"
    padding: "6px"
  suggestion-row:
    textColor: "{colors.light-text}"
    rounded: "{rounded.md}"
    padding: "0 10px"
    height: "34px"
  suggestion-row-selected:
    backgroundColor: "{colors.light-row-selected}"
---

# Design System: Umbrella

## Overview

**Creative North Star: "Windows Glass, Not a Web Page in a Frame"**

The chrome is one 76px frosted slab drawn by GPUI: a tab row that is also the title bar, and a toolbar with a centred omnibox. The DWM backdrop shows through the theme tint. The active tab is a raised capsule that glides on a spring. The page owns everything below 76px, and the chrome never paints over a live page. Density is native-compact: 28px controls, 11 to 13px text, and one icon family at one weight.

Theme tokens drive every colour, radius and spring. Views never hard-code theme colours (`crates/browser-ui/src/style.rs`). The frontmatter records Safari Glass (the light values are historical; Dark is the default and only Safari preset now). The other presets swap the same roles (see Colors).

**Key Characteristics:**
- Glass is a material: tint over the DWM backdrop, turned fully solid by reduced glass or high contrast.
- One accent marks focus, selection, the active capsule edge, an on toggle and a pill's single action.
- Segoe UI Variable Text for words, Segoe Fluent Icons for glyphs. Both ship with Windows 11.
- Springs, not easing curves. Two user styles (Calm, Lively), and they snap when Windows animation effects are off.

Sources of truth:
- Tokens: `crates/browser-core/src/theme/tokens.rs` (`ThemeTokens`: palette, glass, shape, motion, effects).
- Presets: `crates/browser-core/src/theme/presets.rs`.
- Springs: `crates/browser-core/src/spring.rs`.
- Paint colours, sizes and icon code points: `crates/browser-ui/src/style.rs`.
- Components: `crates/browser-ui/src/chrome.rs`.
- Tab motion: `crates/browser-ui/src/motion.rs`.
- Drag, split and theme application: `crates/browser-ui/src/browser.rs`.
- Layout and airspace decision: `docs/decisions/0003-chrome-layout-and-motion.md`.

## Colors

Each preset defines one palette of 13 roles. The chrome derives its paint colours from that palette in `ChromeColors::from_theme`.

### Primary
- **Accent** (`light-accent` / `dark-accent`): The caret, the suggestion selection (accent at 22%, `*-row-selected`), the active capsule's 1px edge (accent at 35%, `*-capsule-edge`), the split button's on state, and the fill of the one action button in a pill.
- **Focus** (`light-focus` / `dark-focus`): A 1px border on the editing omnibox and on the focused master-password field.

### Neutral
- **Glass** (`*-glass`): The glass tint at the glass opacity. It fills the whole 76px chrome over the DWM backdrop.
- **Capsule** (`*-capsule`): `surface_raised` at 90%. Used for the active tab, vault pills and the suggestion panel.
- **Field** (`*-field`): Text at 6%. Fills the omnibox and the password field.
- **Hover** (`*-hover`): Text at 7%. The hover wash on every icon button, inactive tab, caption button and suggestion row.
- **Text / Muted** (`*-text`, `*-text-muted`): Muted text is for the URL path, inactive tabs, glyph prefixes, status counts and placeholders. All chrome text goes muted while the window is inactive.
- **Border** (`*-border`): The single 1px hairline under the chrome.
- **Caption Close** (`caption-close`): Fixed Windows close-button red. It does not change per theme, to match the standard Windows close button.

### Material
| Preset | Backdrop | Tint | Opacity | Spring (k / c) | Radii sm/md/lg |
|---|---|---|---|---|---|
| Safari Glass Dark | Acrylic | `#1c1c1e` | 0.70 | 380 / 30 | 6 / 10 / 14 |
| Catppuccin Mocha | Mica | `#1e1e2e` | 0.80 | 340 / 28 | 6 / 10 / 14 |
| Tokyo Night | Mica | `#1a1b26` | 0.85 | 400 / 34 | 5 / 8 / 12 |
| Nord | Acrylic | `#2e3440` | 0.82 | 360 / 32 | 4 / 8 / 12 |

These four are the only built-ins (the light, OLED and CRT presets were retired at the owner's request). Custom themes are TOML files in `themes/`; Settings > Appearance copies a preset and edits six colour roles (Background also moves the glass tint, Accent also moves the selection fill) plus glass opacity, live on the real chrome, and lists any pair that falls under 4.5:1.

Backdrop maps to `DWMWA_SYSTEMBACKDROP_TYPE`: Acrylic is `Blurred`, Mica is `MicaBackdrop`, and None is `Opaque`. The community presets (Tokyo Night, Nord, Catppuccin) keep their published colours. Their muted text uses the palette's own lighter tone where the usual comment grey would fail AA.

### Named Rules
**The Solid Fallback Rule.** Reduced glass or Windows high contrast calls `ThemeTokens::solid()`. That sets backdrop None, opacity 1.0, blur 0 and no effects. High contrast then replaces every paint colour with `GetSysColor` values: Window, WindowText, Highlight and HighlightText. Hover is Highlight at 35% and selection is Highlight at 50%. No theme hue survives.

**The Dark Desktop Rule.** Every preset must pass 4.5:1 for text and muted text on the chrome. The check runs over the theme's window colour and over pure black, because a dark desktop can sit behind the glass. It covers the solid variant too. The test `every_preset_is_valid_and_meets_aa` (presets.rs) enforces this.

**The Accent Fill Rule.** Accent is a fill or an edge, never text on glass. A pill's action is an accent fill with `on_accent` text, because accent text on the pill would not reach AA.

## Typography

**UI Font:** Segoe UI Variable Text (`style::UI_FONT`)
**Icon Font:** Segoe Fluent Icons (`style::ICON_FONT`)

**Character:** The system's own type and glyphs, so the chrome reads as part of Windows, not as a web page.

### Hierarchy
- **Field** (400, 13px): Omnibox text and suggestion titles.
- **Host** (500, 13px): The address host. The path follows with no gap, at 400 in muted.
- **Body** (400, 12px): Tab titles, pill labels, and suggestion detail (muted, max 260px).
- **Label** (400, 11px): The shield count and the download pill.
- **Action** (600, 12px): Pill action buttons (Fill, Save, Unlock, Create).

### Icons
Segoe Fluent Icons code points are in `style::icon`. They are the same glyphs Windows 11 uses. Glyph sizes:
- 12px: toolbar buttons and suggestion kinds.
- 11px: tab page glyph, new tab, and pill glyphs.
- 10px: caption buttons and the omnibox lock.
- 8px: close glyphs on tabs and pills.

Tabs show a neutral globe glyph until favicons exist.

## Layout

- **Rows.** Tab row 36px (`TAB_ROW`) plus toolbar 40px (`TOOLBAR_ROW`) gives 76px (`CHROME_HEIGHT`). The page `LogicalRect` starts at y = 76.
- **Tab row.** Tabs start at 8px with 2px gaps. Widths are even and clamped to 72 to 220px. The new-tab button (28px) follows the last tab. The empty strip is `WindowControlArea::Drag`. Three 46px caption buttons sit at the right; Max is a real `WindowControlArea::Max`, so snap layouts work. Tabs and the capsule are 28px tall, at top offset 6px.
- **Toolbar.** Back, forward and reload (30px buttons, 2px gap, 8px edge padding) at the left. Bookmark, split, the download pill, the vault pill and the shield count at the right.
- **Omnibox width.** 50% of window width, clamped to [min(280, width), 720]. It is then capped by the room left between both toolbar clusters, with a 200px floor. Room is `width − 2·max(left, right) − 16`, where left = 110 and right = 126. Right grows by 184 while a download pill shows and by 360 while a vault prompt shows. The omnibox stays centred, and it narrows so pills never sit under it.
- **Suggestion panel.** Same x and width as the omnibox, 6px below the chrome, in the page area.
- **Split view.** Two webviews with an 8px GPUI gutter (`SPLIT_GUTTER`, browser.rs). The gutter is the drag handle.

## Elevation & Depth

Depth comes from the material first: tint over the DWM backdrop. A raised capsule sits on top of the glass. Shadows are GPUI presets on a few surfaces only.

### Shadow Vocabulary
- **Raised** (`shadow_sm`): The active tab capsule and vault pills.
- **Floating** (`shadow_lg`): The suggestion panel, the only floating surface.

### Named Rules
**The Airspace Rule.** WebView2 is a child HWND, so GPUI cannot paint over a live page. Nothing overlaps a visible page. While suggestions are open, the page view is hidden. The panel draws in the page area over a snapshot of the page at 30% opacity. Every exit path restores the page: Enter, Escape, click away and Alt-Tab. Prompts live in the toolbar as pills, never over the form.

## Shapes

Corners come from the theme's `shape` scale:
- `radius_md`: tabs, the capsule and the omnibox.
- `radius_lg`: pills and the suggestion panel. Suggestion rows use `radius_lg − 4`, with a minimum of 4px.
- `radius_sm`: the status and download pills.

Icon buttons (6px), action buttons (5px) and the password field (4px) use fixed radii. Borders are limited to these:
- The chrome's bottom hairline.
- The capsule's accent edge.
- The 1px focus border on fields being edited.

## Components

### Tab
- **Rest:** Transparent, muted 12px title, 11px globe glyph, 10px left and 4px right padding, 6px gap.
- **Hover:** Hover wash. The close button (20px, 8px glyph) appears on group hover. It is always visible on the active tab.
- **Active:** Text colour; the capsule below supplies the fill.
- **Audible:** The glyph becomes a 22px mute toggle (volume or mute).
- **Mouse:** Middle-click closes the tab.

### Capsule (signature)
A capsule-fill shape at `radius_md` with a 1px accent edge (35%) and `shadow_sm`. It is a separate layer under the tabs. It glides on its own x and width springs to the active tab's slot. Its first placement snaps.

### Icon Button
Square (30px in the toolbar), 6px radius, glyph centred, hover wash. An on state tints the glyph accent (split). Caption buttons are 46 × 36px; Close uses the caption-close red on hover.

### Omnibox
- **Rest:** A 28px field-fill bar, `radius_md`, content centred. The lock glyph shows for secure pages. The host is Medium, max 70% of the field. The path follows with no gap, in muted. New Tab shows the muted placeholder "Search or enter address".
- **Editing:** The full URL in a text input, a 1px focus border, an accent caret blinking every 500ms, and the placeholder "Search, enter address, or type > for commands".

### Pill (vault fill, save, unlock, password)
28px, capsule fill, `radius_lg`, `shadow_sm`, 10px left and 4px right padding, 6px gap, 12px text. Layout: muted key glyph, label (max 220px; click cycles accounts when there are several), one action button, then a 20px dismiss. The password variant swaps the label for a 150px inset field. That field uses field fill, a 4px radius and a focus border, and shows bullets only. The pill shows only the origin and username, never a password.

### Action Button
22px, accent fill, `on_accent` 12px semibold text, 10px horizontal padding, 5px radius. Hover sets opacity to 0.88. Only one action button appears per pill.

### Status and Download Pills
24px, `radius_sm`, 8px horizontal padding, 5px gap, 11px text. The shield count is muted and not interactive. The download pill (max 180px) shows progress in text colour while active and the file name in muted when done. It has a hover wash, and a click reveals the file.

### Suggestion Panel and Row
- **Panel:** Capsule fill, `radius_lg`, 6px padding, `shadow_lg`, 13px text.
- **Row:** 34px, 10px padding, 10px gap. A muted 12px kind glyph (globe, search, tab, star, history, command), the title, then muted detail.
- **States:** The selected row gets the row-selected fill. Other rows get the hover wash.

### Settings tab
Settings is a tab of its own (`SETTINGS_TAB`, never saved to the session). The capsule glides to it, back/forward/reload fade to 35% and do nothing, and the omnibox reads "Settings". The page area shows a 232px glass sidebar of panes with a gliding selection fill, and a single 720px column of titled groups: one rounded field-fill surface per group, rows split by hairlines, label plus muted detail on the left and the control on the right. Opening the tab fades the page in and lifts the column 16px; switching panes replays a 10px lift and fade. Picking any page tab, opening a foreground tab or editing the address leaves Settings.

### Motion
- **Solver.** One damped spring (`spring.rs`). Substep 1/240s, frame clamp 0.1s, rest at 0.001 distance and 0.01 velocity. Springs request frames only while unsettled.
- **Calm (default).** Raises damping to at least critical, so nothing overshoots. Safari Glass becomes 380 / 39.0.
- **Lively.** Stiffness × 1.1 and damping ratio 0.78: one small, quick settle, no visible wobble.
- **Reduced motion.** With Windows "Animation effects" off (`SPI_GETCLIENTAREAANIMATION`), every spring snaps to its target.
- **Tabs.** New tabs grow from width 0 at their slot. Closed or moved tabs let their neighbours glide.
- **Drag.** It starts after 4px of travel. The dragged tab and its capsule follow the pointer directly, not the spring. When the tab's centre crosses a slot, the tab moves there and the neighbours glide aside on springs.
- **Loading.** The tab's globe glyph pulses in opacity between 0.3 and 1.0 over an 1100ms triangle cycle, separate from the reload icon. With reduced motion it is a static glyph at 0.5.

## Do's and Don'ts

### Do:
- **Do** take every colour from `ChromeColors`, which is derived from theme tokens. Map high contrast to system colours only.
- **Do** keep total chrome at 76px (36 + 40) and give the page everything below it.
- **Do** keep text and muted text at 4.5:1 over both the theme window colour and black, glass and solid. Add any new pair to `contrast_issues`.
- **Do** put new prompts in the toolbar as pills, and add their width to `omnibox_room` so the centred omnibox narrows instead of overlapping them.
- **Do** drive new motion through `browser_core::spring` with the theme's params after `MotionStyle::apply`, and snap under reduced motion.
- **Do** use Segoe Fluent Icons code points from `style::icon` at the sizes above.

### Don't:
- **Don't** draw anything over a visible WebView2. Hide the page and draw over its faded snapshot, as the suggestion panel does.
- **Don't** open a separate popup window for chrome overlays. It deactivates the main window and drops typing (decision 0003, revised).
- **Don't** set accent-coloured text on glass. Use an accent fill with `on_accent`.
- **Don't** spend the accent outside its roles: focus, selection, capsule edge, on toggle, and a pill's single action.
- **Don't** add borders beyond the chrome hairline, the capsule edge and focus borders.
- **Don't** use easing-curve tweens for layout motion, or keep requesting frames after springs settle.
