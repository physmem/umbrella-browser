# 0004: Generated-password offer inside the page

## Decision

- **Generation stays in Rust.** `browser_core::vault::generator` draws from OS randomness through the vault's own `random_bytes`, guarantees one character per enabled class, shuffles, and returns a `Zeroizing<String>`. The page never asks for a password and never receives one in a message.
- **The offer is an injected element, not a toolbar pill.** When a new-password field takes focus, the login bridge (`login_bridge.js`) asks the browser with `{kind: "suggest"}`. If suggestions are on (Settings > Passwords) and the tab is in front, the browser answers with `PostWebMessageAsJson` carrying the theme colours, and the bridge draws a one-row offer 4px under the field. The element is a custom tag with a closed shadow root and `all: initial`, so page CSS cannot restyle it.
- **Accepting fills and forgets.** A click, or ArrowDown then Enter, posts `{kind: "suggest-accept"}`. The browser accepts it only from the tab and origin it last made an offer to. It generates a password, runs a self-contained `ExecuteScript` that re-checks `location` against that origin, fills the focused field and its confirm field, and drops the plaintext when the call returns. Escape dismisses the offer for that field.
- **Saving uses the existing flow.** The generated password lives only in the page's field. When the form is submitted, the bridge's normal capture reports it and the toolbar save pill appears as for any new login. Capture prefers the field Umbrella filled, so a change-password form saves the new password, not the current one. After the fill, the offer turns into a short note saying the save offer comes on submit.
- **Which fields count.** `autocomplete="new-password"`, or two or more password fields in the form, or a name, id, placeholder or label that reads as new, confirm, create, register or sign-up. `current-password`, or anything named current or old, never gets an offer. The confirm field is the next password field in the form. If the user started on the confirm field and the first field is empty, the first field is the confirm field.

## Why not the toolbar pill

The Airspace Rule (DESIGN.md) keeps GPUI from painting over a live WebView2, so the pill is the usual home for vault prompts. It is the wrong home here:

- The offer is about one field. A pill 40 to 600px away from that field, appearing on focus, reads as unrelated chrome. Users of every other browser expect the offer under the field.
- The airspace limit is on GPUI, not on page content. An element the page itself lays out sits under the field, scrolls with it, and never overlaps the chrome.
- The pill's strength is that the page cannot touch it. The offer gives nothing away: a page that fakes an accept only gets a random password put into its own focused field, which it would receive on submit anyway. Saving to the vault still needs the user's click on the trusted save pill. Fill and save prompts that carry saved accounts stay in the toolbar.

## Risks accepted

- The page can see, hide, restyle around or cover the offer, and can watch the field fill. It cannot read anything from the vault through it.
- A page can post `suggest` on any focus. It only gets an offer for its own field, and only while suggestions are on.
- The password crosses COM once as UTF-16 for the fill, as decision 0002 already accepts for saved-password fill.
- Submit capture can miss a form that never submits, clicks a button or presses Enter (for example a sign-up driven only by a custom element). The password is then in the page but not in the vault. Reopen with an "Save now" action on the note if this shows up on real sites.

## Deferred

- Showing or copying the generated password before it is used.
- Offers inside cross-origin iframes (the bridge only runs in the top document; see 0002).
