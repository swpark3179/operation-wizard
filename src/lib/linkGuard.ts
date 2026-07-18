// External-link guard for previewed HTML (D76). Self-contained HTML previews
// (operation-guide `.html`, RAG '검색 결과' panel) run inside sandboxed iframes
// (`allow-scripts`, no same-origin), and markdown links render in the top frame.
// A plain `<a href="http://…">` click would navigate the pane (iframe) or the
// whole app (top frame) away — turning Operation Wizard into a browser with no
// way back. This module produces a capture-phase click guard injected into the
// iframe srcdoc: it intercepts external links, prevents the in-frame/top
// navigation, and posts the URL to the parent, which opens it in the OS browser
// via `openExternal` (opener plugin). Pure module — no Tauri imports.

/** postMessage type tag from the injected iframe guard to the parent listener.
 * Kept in sync with the literal inside LINK_GUARD_SCRIPT (a string, so it can't
 * import this constant). */
export const OW_OPEN_URL = "ow-open-url";

/** True for URLs we route to the OS browser/mail client. Everything else
 * (`#anchor`, relative, `javascript:`, `data:`, `blob:`) is deliberately left
 * alone — never opened externally. Shared by the parent listener (re-validation)
 * and the markdown `a` override. */
export function isExternalUrl(raw: string): boolean {
  return /^\s*(https?:|mailto:)/i.test(raw);
}

/** Inline guard script injected into every previewed HTML srcdoc. Binds to
 * `document` in capture phase (so it works regardless of DOM readiness or where
 * the <script> sits), intercepts external-link activation (click + Enter),
 * prevents navigation, and forwards the URL to the parent window. Fragment,
 * relative and `javascript:` links are left to the sandbox. Must not contain a
 * literal `</script>`. */
export const LINK_GUARD_SCRIPT = `(function () {
  function open(a, e) {
    var href = a && a.getAttribute && a.getAttribute('href');
    if (!href) return;
    if (/^\\s*(https?:|mailto:)/i.test(href)) {
      e.preventDefault();
      try { window.parent.postMessage({ type: 'ow-open-url', url: href.trim() }, '*'); } catch (_) {}
    }
    // '#', relative, javascript: — leave to the sandbox
  }
  document.addEventListener('click', function (e) {
    var a = e.target && e.target.closest && e.target.closest('a[href]');
    if (a) open(a, e);
  }, true);
  document.addEventListener('keydown', function (e) {
    if (e.key !== 'Enter') return;
    var a = e.target && e.target.closest && e.target.closest('a[href]');
    if (a) open(a, e);
  }, true);
})();`;

/** Insert the guard <script> at the earliest safe point of an HTML document so
 * it runs even for complete pass-through docs and before any late `<head>` CSP
 * <meta>: before the first `</head>`, else before the opening `<body …>`, else
 * before `</body>`, else appended. Case-insensitive; leaves the rest intact. */
export function withLinkGuard(html: string): string {
  const tag = `<script>${LINK_GUARD_SCRIPT}</script>`;
  const insertBefore = (re: RegExp): string | null => {
    const m = re.exec(html);
    if (!m) return null;
    return html.slice(0, m.index) + tag + html.slice(m.index);
  };
  return (
    insertBefore(/<\/head\s*>/i) ??
    insertBefore(/<body\b[^>]*>/i) ??
    insertBefore(/<\/body\s*>/i) ??
    html + tag
  );
}
