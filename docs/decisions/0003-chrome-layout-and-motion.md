# 0003: Chrome layout, overlays and motion

## Decision

- **Layout.** Safari-style top chrome, drawn by GPUI with a custom title bar: a 36 px tab row that doubles as the drag region and holds the window controls, and a 40 px toolbar with back, forward, reload, a centered omnibox and status pills. Pages get everything below 76 px.
- **Airspace.** WebView2 is a child HWND, so GPUI cannot paint over a page. Nothing overlaps a visible page. While the omnibox or command palette has suggestions, the page view is hidden and the list is drawn in the page area on the window glass, aligned under the omnibox. Vault fill and save prompts sit in the toolbar as a pill, not over the form.
  - *Revised 2026-11-19:* the first version drew the list in a separate borderless popup window. Opening it deactivated the main window, which GPUI reports as a blur, so typing was lost and the popup closed itself. The in-window panel has none of those problems.
- **Split view.** Two webviews side by side with an 8 px GPUI gutter between them. The gutter is the drag handle; panes round their shared edge with `LogicalRect::to_physical` so they never gap.
- **Motion.** One spring solver (`browser_core::spring`). A `MotionStyle` setting picks Calm (theme damping, little or no overshoot) or Lively (lower damping, visible bounce). Windows "Animation effects" off forces snap-to-target either way.
- **Material.** The window uses the theme's DWM backdrop; chrome surfaces are the theme tint at its glass opacity. Reduced-glass and Windows high contrast switch to opaque surfaces.
- **Type and icons.** Segoe UI Variable for text and Segoe Fluent Icons for glyphs, both shipped with Windows 11, so the chrome reads as native rather than web.

## Council

- **Systems:** Hiding a view while the list is open costs nothing; it is not suspended. Springs request frames only while unsettled, so an idle window draws nothing.
- **Security:** Prompts never show passwords; the pill shows the origin and username. Fill only runs on an explicit click or Enter.
- **UX:** 76 px total chrome is close to Safari's two-row layout; a compact one-row mode is a later option. Lively motion is opt-in.
- **Skeptic:** Hiding the page while typing must be undone on every exit path (Enter, Escape, click away, Alt-Tab). Custom title bars must keep Windows snap layouts: the maximize button is a real `WindowControlArea::Max`.

## Deferred

- Compact single-row mode.
- Composition-hosted WebView2 (would allow drawing over pages); revisit if overlay windows feel wrong.
