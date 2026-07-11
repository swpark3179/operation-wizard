import { useEffect, useMemo, useRef, useState } from "react";
import { Eye, Code2, FileText, ListTree } from "lucide-react";
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
  // Heading outline for the markdown preview (D58): extracted from the
  // rendered DOM (no slugs/anchors — jumps go by element index), so Korean
  // headings, duplicates and headings inside code fences all behave.
  const previewRef = useRef<HTMLDivElement | null>(null);
  const [outline, setOutline] = useState<{ level: number; text: string }[]>([]);
  const [outlineOpen, setOutlineOpen] = useState(false);

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

  // Extract the outline after the preview commits (ReactMarkdown renders
  // synchronously; the async mermaid swaps add no headings). `loading` is a
  // dep because the preview div only mounts once loading flips false — a
  // separate render where content/mode/path don't change — so without it the
  // query would run against a not-yet-mounted previewRef.
  useEffect(() => {
    setOutlineOpen(false);
    if (loading || mode !== "preview" || !path || !isMarkdown(path) || content === null) {
      setOutline([]);
      return;
    }
    const els = previewRef.current?.querySelectorAll<HTMLElement>("h1, h2, h3");
    setOutline(
      Array.from(els ?? []).map((el, i) => ({
        level: Number(el.tagName[1]),
        text: el.textContent?.trim() || `제목 ${i + 1}`,
      })),
    );
  }, [content, mode, path, loading]);

  // Jump by index, re-querying at click time (immune to re-renders).
  const jumpTo = (index: number) => {
    previewRef.current
      ?.querySelectorAll<HTMLElement>("h1, h2, h3")
      [index]?.scrollIntoView({ behavior: "smooth", block: "start" });
    setOutlineOpen(false);
  };

  if (!path) {
    return (
      <div className="flex flex-1 items-center justify-center text-[12.5px] text-ink-faint">
        파일을 선택하면 내용이 여기에 표시됩니다.
      </div>
    );
  }

  const showPreviewToggle = isHtml(path) || isMarkdown(path);

  const showOutline = mode === "preview" && isMarkdown(path) && outline.length > 0;

  return (
    <div className="relative flex min-w-0 flex-1 flex-col">
      {/* file bar */}
      <div className="flex h-9 shrink-0 items-center gap-2 border-b border-line bg-panel px-3">
        <FileText size={13} className="shrink-0 text-ink-soft" />
        <span className="truncate font-mono text-[12px] text-ink-muted">{path}</span>
        <div className="flex-1" />
        {showOutline && (
          <button
            type="button"
            onClick={() => setOutlineOpen((o) => !o)}
            title="목차"
            className={
              "grid h-6 w-6 shrink-0 place-items-center rounded-md border border-line transition-colors " +
              (outlineOpen ? "bg-accent-tint text-accent" : "text-ink-soft hover:bg-subtle")
            }
          >
            <ListTree size={13} />
          </button>
        )}
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
            <div ref={previewRef} className="mx-auto max-w-[760px] px-6 py-5">
              <MarkdownView content={content} />
            </div>
          ) : (
            <pre className="whitespace-pre-wrap px-4 py-3 font-mono text-[12px] leading-[1.6] text-ink">
              {content}
            </pre>
          )
        )}
      </div>

      {/* outline popover (D58) — same pattern as ChatPanel's history popover:
          a transparent full-cover close layer + an anchored dropdown. */}
      {outlineOpen && showOutline && (
        <>
          <button
            type="button"
            aria-label="목차 닫기"
            className="absolute inset-0 z-10 cursor-default"
            onClick={() => setOutlineOpen(false)}
          />
          <div className="absolute right-2 top-[38px] z-20 max-h-[60%] w-[240px] overflow-y-auto rounded-xl border border-line-strong bg-elevated p-1.5 shadow-lg">
            {outline.map((h, i) => (
              <button
                key={i}
                type="button"
                onClick={() => jumpTo(i)}
                title={h.text}
                style={{ paddingLeft: 8 + (h.level - 1) * 12 }}
                className="block w-full truncate rounded-md py-1 pr-2 text-left text-[12px] text-ink-muted transition-colors hover:bg-subtle hover:text-ink"
              >
                {h.text}
              </button>
            ))}
          </div>
        </>
      )}
    </div>
  );
}
