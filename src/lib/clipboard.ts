/** Copy text to the clipboard. Prefers the async Clipboard API; falls back to
 * the legacy execCommand path for WebView2 configurations where the API is
 * unavailable or permission-gated. Returns whether the copy succeeded. */
export async function copyText(text: string): Promise<boolean> {
  try {
    await navigator.clipboard.writeText(text);
    return true;
  } catch {
    // fall through to the legacy path
  }
  try {
    const ta = document.createElement("textarea");
    ta.value = text;
    ta.style.position = "fixed";
    ta.style.opacity = "0";
    document.body.appendChild(ta);
    ta.select();
    const ok = document.execCommand("copy");
    ta.remove();
    return ok;
  } catch {
    return false;
  }
}

/** Copy rich HTML to the clipboard as BOTH `text/html` (so pasting into a
 * rich-text editor — DC Manager, Confluence, mail — keeps the markup) and a
 * `text/plain` fallback. Prefers the async Clipboard API's `ClipboardItem`;
 * falls back to selecting a hidden contenteditable holder + execCommand("copy")
 * for WebView2 configurations where the API is unavailable/permission-gated
 * (the browser derives text/html from the rich selection). Returns success. */
export async function copyHtml(html: string, plain?: string): Promise<boolean> {
  const text = plain ?? html;
  try {
    if (
      typeof ClipboardItem !== "undefined" &&
      navigator.clipboard &&
      "write" in navigator.clipboard
    ) {
      const item = new ClipboardItem({
        "text/html": new Blob([html], { type: "text/html" }),
        "text/plain": new Blob([text], { type: "text/plain" }),
      });
      await navigator.clipboard.write([item]);
      return true;
    }
  } catch {
    // fall through to the legacy selection path
  }
  try {
    const holder = document.createElement("div");
    holder.setAttribute("contenteditable", "true");
    holder.innerHTML = html;
    holder.style.position = "fixed";
    holder.style.left = "-9999px";
    holder.style.opacity = "0";
    document.body.appendChild(holder);
    const range = document.createRange();
    range.selectNodeContents(holder);
    const sel = window.getSelection();
    sel?.removeAllRanges();
    sel?.addRange(range);
    const ok = document.execCommand("copy");
    sel?.removeAllRanges();
    holder.remove();
    return ok;
  } catch {
    return false;
  }
}
