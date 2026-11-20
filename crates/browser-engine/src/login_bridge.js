// Injected into every top-level document. Reports login forms and submitted
// credentials to the browser. It never receives saved passwords.
(() => {
  if (window.top !== window || !window.chrome || !window.chrome.webview) return;
  const post = (msg) => window.chrome.webview.postMessage({ umbrella: "login", ...msg });
  const passwordField = () => document.querySelector("input[type=password]");
  const textLike = "input:not([type=hidden]):not([type=password]):not([type=checkbox])" +
    ":not([type=radio]):not([type=submit]):not([type=button])";

  const usernameFor = (field) => {
    const scope = field.form || document;
    const before = [...scope.querySelectorAll(textLike)]
      .filter((i) => field.compareDocumentPosition(i) & Node.DOCUMENT_POSITION_PRECEDING);
    const pick = before.pop();
    return pick ? pick.value : "";
  };

  // The field Umbrella last filled with a generated password. Capture
  // prefers it, so a change-password form saves the new password, not the old.
  let generated = null;

  let lastSent = "";
  const capture = () => {
    const field = generated && generated.isConnected && generated.value
      ? generated : passwordField();
    if (!field || !field.value) return;
    const username = usernameFor(field);
    const key = username + "\u0000" + field.value;
    if (key === lastSent) return;
    lastSent = key;
    post({ kind: "submit", username, password: field.value });
  };

  let announced = false;
  let observer = null;
  const scan = () => {
    if (announced || !passwordField()) return;
    announced = true;
    if (observer) observer.disconnect();
    post({ kind: "form" });
  };

  // ---- Generated-password offer ------------------------------------------
  // Focusing a new-password field asks the browser; it answers "offer" (with
  // theme colours) only while suggestions are on. Accepting asks the browser
  // to generate and fill. The password is made in Rust and never posted here.
  const attrs = (el) => [el.name, el.id, el.placeholder, el.getAttribute("aria-label")]
    .join(" ");
  const wantsNew = (el) => {
    if (!(el instanceof HTMLInputElement) || el.type !== "password") return false;
    if (el.readOnly || el.disabled) return false;
    const ac = (el.getAttribute("autocomplete") || "").toLowerCase();
    if (ac.includes("new-password")) return true;
    if (ac.includes("current-password") || /current|old/i.test(attrs(el))) return false;
    const scope = el.form || document;
    if (scope.querySelectorAll("input[type=password]").length >= 2) return true;
    return /new|confirm|create|register|sign.?up|repeat|retype/i.test(attrs(el));
  };

  let host = null, root = null, row = null, target = null, theme = null;
  let selected = false, filledTimer = 0;
  const dismissed = new WeakSet();

  const glyph = (code) => `<span class="g">${code}</span>`;
  const render = (filled) => {
    if (!root) {
      host = document.createElement("umbrella-password-offer");
      root = host.attachShadow({ mode: "closed" });
      root.innerHTML = `<style>
        :host { all: initial; }
        .row { box-sizing: border-box; display: flex; align-items: center; gap: 10px;
          height: 34px; padding: 0 10px; max-width: 480px;
          border-radius: 10px; cursor: default; user-select: none; white-space: nowrap;
          font: 400 12px "Segoe UI Variable Text", "Segoe UI", system-ui, sans-serif; }
        .row.act { cursor: pointer; }
        .g { font: 12px "Segoe Fluent Icons", "Segoe MDL2 Assets"; flex: none; }
        .t { font-weight: 600; flex: none; }
        .d { overflow: hidden; text-overflow: ellipsis; min-width: 0; }
        .card { box-sizing: border-box; border-radius: 14px; padding: 4px; }
      </style><div class="card"><div class="row" role="button"></div></div>`;
      row = root.querySelector(".row");
      const card = root.querySelector(".card");
      // Keep focus in the field, so the fill knows which field is meant.
      card.addEventListener("mousedown", (e) => e.preventDefault());
      row.addEventListener("mouseenter", () => { row.dataset.hover = "1"; paint(); });
      row.addEventListener("mouseleave", () => { delete row.dataset.hover; paint(); });
      row.addEventListener("click", (e) => { e.stopPropagation(); accept(); });
    }
    row.classList.toggle("act", !filled);
    row.dataset.filled = filled ? "1" : "";
    row.innerHTML = filled
      ? `${glyph("")}<span class="t">Strong password filled</span>` +
        `<span class="d">Umbrella offers to save it when you submit</span>`
      : `${glyph("")}<span class="t">Use a strong password</span>` +
        `<span class="d">Made by Umbrella, saved to your vault</span>`;
    paint();
    host.style.cssText = "all: initial !important; position: absolute !important;" +
      "z-index: 2147483647 !important; display: block !important;";
    if (!host.isConnected) document.documentElement.appendChild(host);
    place();
  };

  const paint = () => {
    if (!row || !theme) return;
    const card = root.querySelector(".card");
    card.style.background = theme.surface;
    card.style.boxShadow = `0 0 0 1px ${theme.border}, 0 8px 24px rgba(0,0,0,.28)`;
    const lit = !row.dataset.filled && (selected || row.dataset.hover);
    row.style.background = lit ? (selected ? theme.selection : theme.hover) : "transparent";
    row.style.color = theme.text;
    for (const el of row.querySelectorAll(".g, .d")) el.style.color = theme.muted;
  };

  const place = () => {
    if (!host || !target || !target.isConnected) return hide();
    const r = target.getBoundingClientRect();
    // At least as wide as the field, so the two read as one control.
    root.querySelector(".card").style.minWidth = `${Math.max(r.width, 268)}px`;
    host.style.setProperty("left", `${r.left + scrollX}px`, "important");
    host.style.setProperty("top", `${r.bottom + scrollY + 4}px`, "important");
  };

  const hide = () => {
    clearTimeout(filledTimer);
    selected = false;
    if (host) host.remove();
  };
  const showing = () => host && host.isConnected;

  const accept = () => {
    if (!target || !showing() || row.dataset.filled) return;
    post({ kind: "suggest-accept" });
  };

  document.addEventListener("focusin", (e) => {
    const el = e.target;
    if (el !== target) hide();
    if (!wantsNew(el) || el.value || dismissed.has(el)) return;
    target = el;
    post({ kind: "suggest" });
  }, true);
  document.addEventListener("focusout", (e) => {
    if (e.target === target) hide();
  }, true);
  document.addEventListener("input", (e) => {
    if (e.target === target && showing() && !row.dataset.filled) hide();
  }, true);
  document.addEventListener("umbrella-generated", (e) => {
    generated = e.target instanceof HTMLInputElement ? e.target : null;
    if (e.target !== target) return;
    render(true);
    filledTimer = setTimeout(hide, 6000);
  }, true);
  addEventListener("scroll", () => showing() && place(), true);
  addEventListener("resize", () => showing() && place());
  // Registered before the Enter capture below, so accepting by keyboard
  // never also reads as a submit.
  document.addEventListener("keydown", (e) => {
    if (!showing() || e.target !== target || row.dataset.filled) return;
    if (e.key === "ArrowDown" || e.key === "ArrowUp") {
      selected = e.key === "ArrowDown";
      paint();
    } else if (e.key === "Enter" && selected) {
      accept();
    } else if (e.key === "Escape") {
      dismissed.add(target);
      hide();
    } else {
      return;
    }
    e.preventDefault();
    e.stopImmediatePropagation();
  }, true);
  window.chrome.webview.addEventListener("message", (e) => {
    const m = e.data;
    if (!m || m.umbrella !== "login" || m.kind !== "offer") return;
    if (!target || document.activeElement !== target || target.value) return;
    theme = m.theme;
    render(false);
  });

  document.addEventListener("submit", capture, true);
  document.addEventListener("keydown", (e) => {
    if (e.key === "Enter" && e.target && e.target.type === "password") capture();
  }, true);
  document.addEventListener("click", (e) => {
    const t = e.target;
    if (t && t.closest && t.closest("button, input[type=submit]")) capture();
  }, true);

  const start = () => {
    scan();
    if (announced) return;
    observer = new MutationObserver(scan);
    observer.observe(document.documentElement, { childList: true, subtree: true });
  };
  if (document.readyState === "loading") {
    document.addEventListener("DOMContentLoaded", start, { once: true });
  } else {
    start();
  }
})();
