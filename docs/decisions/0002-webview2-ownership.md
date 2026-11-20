# 0002: WebView2 ownership, events and the login bridge

## Decision

- **One thread.** Every WebView2 object lives on the UI thread, which is a COM STA with a message loop. Nothing in `browser-engine` is `Send`; it uses `Rc` and `RefCell`, not locks.
- **Ownership.** `Profile` owns one `ICoreWebView2Environment` (one user data folder per workspace). `TabView` owns one controller and its webview, and calls `Close()` on drop. Handlers capture only an `EventQueue`, never the `TabView`, so there is no reference cycle and dropping a tab frees it.
- **Async, never pumped.** Environment and controller creation use completion callbacks delivered by the normal message loop. No nested `wait_with_pump`, so GPUI never re-enters itself.
- **Events out, commands in.** Handlers push `EngineEvent`s onto a queue and call an optional waker. The UI drains the queue on its own schedule. Handlers never call back into UI state.
- **Lazy and hibernated.** A tab gets a controller only when it becomes visible. Hidden tabs are hidden then `TrySuspend`ed after a short idle; long-hidden tabs are discarded entirely and come back from their `SavedTab`. Tabs playing audio are never suspended.
- **Login bridge.** A document-created script reports login forms and submitted credentials with `postMessage`. Only top-level `WebMessageReceived` is handled, and the origin comes from `args.Source()` in Rust, never from the message body. Fill runs a self-contained `ExecuteScript` that re-checks `location` against the expected origin before touching inputs.

## Council

- **Systems:** Controller creation is ~100 ms and a renderer is tens of MB, so lazy creation and discard dominate RAM. The queue keeps COM callbacks O(1).
- **Security:** Any page can call `postMessage`, so messages are untrusted. A forged "submit" can at worst raise a save prompt for the page's own origin. Passwords cross COM as plain `PWSTR` once during fill and once on submit; that copy cannot be zeroized and is accepted. SmartScreen stays enabled; only sync, first-run and shopping extras are turned off.
- **UX:** Fill waits for an explicit user choice. Suspend never touches the visible or audible tab.
- **Skeptic:** WebView2 runtime may be missing; environment creation must surface a clear error. `TrySuspend` fails on a visible webview, so visibility goes off first. Navigation can race a fill prompt; the in-page origin check covers it.

## Deferred

- Cross-origin iframe login forms (needs `FrameWebMessageReceived`). Reopen if common sites break.
- Isolated script worlds for userscripts. WebView2 has no API for it today.
