// Toggles a reader view over the page: the main article, restyled, in a
// closed shadow root so the page's CSS cannot reach it. The page itself is
// left untouched underneath. Returns true when the reader is now showing.
((bg, fg, muted, accent) => {
  const old = document.getElementById("umbrella-reader");
  if (old) {
    old.remove();
    document.documentElement.style.overflow = "";
    return false;
  }
  // Readability-style guess: the element whose own paragraphs hold the most text.
  const scores = new Map();
  for (const p of document.querySelectorAll("p")) {
    const len = p.innerText.trim().length;
    if (len < 40 || !p.parentElement) continue;
    scores.set(p.parentElement, (scores.get(p.parentElement) || 0) + len);
  }
  let best = null;
  let top = 0;
  for (const [el, score] of scores) {
    if (score > top) { best = el; top = score; }
  }
  if (!best || top < 400) return null;
  // Sites often split an article into one block per section; climb while
  // the parent holds clearly more of the page's paragraph text.
  const textIn = (el) =>
    [...el.querySelectorAll("p")].reduce((n, p) => n + p.innerText.trim().length, 0);
  let article = best.closest("article") || best;
  for (let i = 0; i < 6 && article.parentElement && article.parentElement !== document.body; i++) {
    if (textIn(article.parentElement) < textIn(article) * 1.3) break;
    article = article.parentElement;
  }
  const clone = article.cloneNode(true);
  clone.querySelectorAll(
    "script,style,link,iframe,form,button,input,select,textarea,nav,aside,footer,noscript,svg," +
    "[aria-hidden=true],[role=complementary],[role=navigation],sup.reference,.mw-editsection,.navbox"
  ).forEach((n) => n.remove());
  const noise = /(^|[-_\s])(edit|share|social|comment|related|promo|newsletter|advert|sponsor)/i;
  clone.querySelectorAll("[class]").forEach((n) => {
    if (noise.test(n.getAttribute("class") || "")) n.remove();
  });
  for (const n of clone.querySelectorAll("*")) {
    for (const attr of [...n.attributes]) {
      if (!["href", "src", "alt", "title"].includes(attr.name)) n.removeAttribute(attr.name);
    }
    if (n.tagName === "A" && /^\s*javascript:/i.test(n.getAttribute("href") || "")) n.removeAttribute("href");
  }
  const host = document.createElement("div");
  host.id = "umbrella-reader";
  host.setAttribute("style", "position:fixed;inset:0;z-index:2147483647;overflow:auto;background:" + bg);
  const root = host.attachShadow({ mode: "closed" });
  const style = document.createElement("style");
  style.textContent = `
    :host { all: initial; }
    main { max-width: 680px; margin: 0 auto; padding: 56px 24px 96px; color: ${fg};
           font: 20px/1.65 Georgia, "Iowan Old Style", serif; }
    h1 { font: 600 34px/1.2 "Segoe UI Variable Display", "Segoe UI", sans-serif; margin: 0 0 8px; }
    .site { font: 14px "Segoe UI", sans-serif; color: ${muted}; margin-bottom: 40px; }
    h2, h3 { font-family: "Segoe UI", sans-serif; line-height: 1.3; }
    img, video, figure { max-width: 100%; height: auto; }
    figcaption { font-size: 15px; color: ${muted}; }
    a { color: ${accent}; }
    pre, code { font: 15px "Cascadia Mono", Consolas, monospace; white-space: pre-wrap; }
    blockquote { margin: 0; padding-left: 20px; border-left: 3px solid ${muted}; color: ${muted}; }
  `;
  const main = document.createElement("main");
  const title = document.createElement("h1");
  title.textContent = (article.querySelector("h1") || {}).textContent || document.title;
  const site = document.createElement("div");
  site.className = "site";
  site.textContent = location.hostname.replace(/^www\./, "") + " · Esc to close";
  clone.querySelector("h1")?.remove();
  main.append(title, site, clone);
  root.append(style, main);
  document.documentElement.append(host);
  document.documentElement.style.overflow = "hidden";
  const close = (e) => {
    if (e.key !== "Escape") return;
    host.remove();
    document.documentElement.style.overflow = "";
    document.removeEventListener("keydown", close, true);
  };
  document.addEventListener("keydown", close, true);
  host.tabIndex = -1;
  host.focus();
  return true;
})
