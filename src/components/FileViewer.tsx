import { useEffect, useMemo, useState } from "react";
import { Eye, Code2, FileText } from "lucide-react";
import { MarkdownView } from "./Markdown";
import { readFile } from "../lib/api";

function isHtml(path: string): boolean {
  return /\.html?$/i.test(path);
}

function isMarkdown(path: string): boolean {
  return /\.(md|markdown)$/i.test(path);
}

/** Minimal port of Open Design's `buildSrcdoc`: full documents pass through;
 * fragments get a doctype shell. Rendered in a sandboxed iframe (no same-origin). */
function buildSrcdoc(content: string): string {
  if (/^\s*(<!doctype|<html)/i.test(content)) return content;
  return `<!doctype html><html><head><meta charset="utf-8"><meta name="viewport" content="width=device-width, initial-scale=1"></head><body>${content}</body></html>`;
}

export function FileViewer({
  path,
  refreshNonce,
}: {
  path: string | null;
  /** Bumped to re-read the open file (e.g. a later workflow step rewrote it). */
  refreshNonce?: number;
}) {
  const [content, setContent] = useState<string | null>(null);
  const [error, setError] = useState<string | null>(null);
  const [loading, setLoading] = useState(false);
  const [mode, setMode] = useState<"preview" | "source">("preview");

  useEffect(() => {
    if (!path) {
      setContent(null);
      setError(null);
      return;
    }
    let cancelled = false;
    setLoading(true);
    setError(null);
    setMode(isHtml(path) || isMarkdown(path) ? "preview" : "source");
    readFile(path)
      .then((c) => {
        if (!cancelled) setContent(c);
      })
      .catch((e) => {
        if (!cancelled) {
          setError(String(e));
          setContent(null);
        }
      })
      .finally(() => {
        if (!cancelled) setLoading(false);
      });
    return () => {
      cancelled = true;
    };
  }, [path, refreshNonce]);

  const srcdoc = useMemo(
    () => (content !== null && path && isHtml(path) ? buildSrcdoc(content) : ""),
    [content, path],
  );

  if (!path) {
    return (
      <div className="flex flex-1 items-center justify-center text-[12.5px] text-ink-faint">
        파일을 선택하면 내용이 여기에 표시됩니다.
      </div>
    );
  }

  const showPreviewToggle = isHtml(path) || isMarkdown(path);

  return (
    <div className="flex min-w-0 flex-1 flex-col">
      {/* file bar */}
      <div className="flex h-9 shrink-0 items-center gap-2 border-b border-line bg-panel px-3">
        <FileText size={13} className="shrink-0 text-ink-soft" />
        <span className="truncate font-mono text-[12px] text-ink-muted">{path}</span>
        <div className="flex-1" />
        {showPreviewToggle && (
          <div className="flex overflow-hidden rounded-md border border-line">
            <button
              type="button"
              onClick={() => setMode("preview")}
              className={
                "inline-flex items-center gap-1 px-2 py-1 text-[11.5px] " +
                (mode === "preview" ? "bg-accent-tint text-accent" : "text-ink-soft hover:bg-subtle")
              }
            >
              <Eye size={12} /> 미리보기
            </button>
            <button
              type="button"
              onClick={() => setMode("source")}
              className={
                "inline-flex items-center gap-1 border-l border-line px-2 py-1 text-[11.5px] " +
                (mode === "source" ? "bg-accent-tint text-accent" : "text-ink-soft hover:bg-subtle")
              }
            >
              <Code2 size={12} /> 소스
            </button>
          </div>
        )}
      </div>

      {/* body */}
      <div className="min-h-0 flex-1 overflow-auto bg-app">
        {loading && <div className="px-4 py-3 text-[12.5px] text-ink-soft">불러오는 중…</div>}
        {error && <div className="px-4 py-3 text-[12.5px] text-bad">{error}</div>}
        {!loading && !error && content !== null && (
          mode === "preview" && isHtml(path) ? (
            <iframe
              title={path}
              sandbox="allow-scripts"
              srcDoc={srcdoc}
              className="h-full w-full border-0 bg-white"
            />
          ) : mode === "preview" && isMarkdown(path) ? (
            <div className="mx-auto max-w-[760px] px-6 py-5">
              <MarkdownView content={content} />
            </div>
          ) : (
            <pre className="whitespace-pre-wrap px-4 py-3 font-mono text-[12px] leading-[1.6] text-ink">
              {content}
            </pre>
          )
        )}
      </div>
    </div>
  );
}
