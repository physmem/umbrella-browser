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

  let lastSent = "";
  const capture = () => {
    const field = passwordField();
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
