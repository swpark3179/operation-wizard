// Canvas 다이어그램 tab (design D58): every ```mermaid fence found in the
// workflow's markdown artifacts, rendered as a card gallery with an enlarge
// modal. The scan is lazy — this component mounts only while the tab is
// active — and re-runs when a workflow step writes a file (refreshNonce) or on
// the manual re-scan button. Rendering reuses the markdown preview's
// MermaidDiagram (same loader, same per-diagram failure fallback).

import { useEffect, useState } from "react";
import { Network, RefreshCw, X, ZoomIn, ZoomOut } from "lucide-react";
import { MermaidDiagram } from "./Markdown";
import { readFile } from "../lib/api";
import {
  extractMermaidBlocks,
  isMarkdownFile,
  joinWorkdirPath,
  type ArtifactDef,
} from "../lib/artifacts";

interface DiagramItem {
  key: string;
  /** Source label, e.g. "plan.md · 2" (file basename + index within it). */
  label: string;
  code: string;
  /** How many additional places carry the identical diagram (dedupe count). */
  extra: number;
}

function basename(p: string): string {
  const parts = p.split(/[\\/]/).filter(Boolean);
  return parts[parts.length - 1] ?? p;
}

export function DiagramGallery({
  workdir,
  artifacts,
  refreshNonce,
}: {
  workdir: string;
  /** The workflow's document artifacts — only markdown ones are scanned. */
  artifacts: ArtifactDef[];
  /** Bumped when a workflow step writes a file → re-scan. */
  refreshNonce?: number;
}) {
  const [items, setItems] = useState<DiagramItem[] | null>(null); // null = scanning
  const [sources, setSources] = useState(0);
  const [zoom, setZoom] = useState<DiagramItem | null>(null);
  // Enlarge-modal magnification. CSS `zoom` (not transform) so the scaled
  // diagram participates in layout and the scroll container tracks it.
  const [scale, setScale] = useState(1);
  const [localNonce, setLocalNonce] = useState(0);

  const openZoom = (item: DiagramItem) => {
    setScale(1);
    setZoom(item);
  };
  const zoomBy = (dir: 1 | -1) =>
    setScale((s) => Math.min(4, Math.max(0.25, Math.round((s + dir * 0.25) * 100) / 100)));

  useEffect(() => {
    let cancelled = false;
    setItems(null);
    const files = artifacts.filter((a) => isMarkdownFile(a.file));
    void Promise.allSettled(
      files.map((a) => readFile(joinWorkdirPath(workdir, a.file))),
    ).then((results) => {
      if (cancelled) return;
      // Missing / oversized / unreadable files simply reject → skipped.
      const byCode = new Map<string, DiagramItem>();
      let sourceCount = 0;
      results.forEach((r, i) => {
        if (r.status !== "fulfilled") return;
        const blocks = extractMermaidBlocks(r.value);
        if (blocks.length > 0) sourceCount += 1;
        blocks.forEach((code, j) => {
          const trimmed = code.trim();
          if (!trimmed) return;
          const prev = byCode.get(trimmed);
          if (prev) {
            prev.extra += 1;
          } else {
            byCode.set(trimmed, {
              key: `${files[i].stepId}:${j}`,
              label: `${basename(files[i].file)} · ${j + 1}`,
              code: trimmed,
              extra: 0,
            });
          }
        });
      });
      setItems([...byCode.values()]);
      setSources(sourceCount);
    });
    return () => {
      cancelled = true;
    };
  }, [workdir, artifacts, refreshNonce, localNonce]);

  // Close the enlarge modal with Escape while it is open.
  useEffect(() => {
    if (!zoom) return;
    const onKey = (e: KeyboardEvent) => {
      if (e.key === "Escape") setZoom(null);
    };
    window.addEventListener("keydown", onKey);
    return () => window.removeEventListener("keydown", onKey);
  }, [zoom]);

  return (
    <div className="flex min-h-0 flex-1 flex-col">
      {/* gallery bar */}
      <div className="flex h-9 shrink-0 items-center gap-2 border-b border-line bg-panel px-3">
        <Network size={13} className="shrink-0 text-ink-soft" />
        <span className="truncate text-[12px] text-ink-muted">
          {items === null
            ? "문서를 읽는 중…"
            : items.length > 0
              ? `다이어그램 ${items.length}개 · 문서 ${sources}개에서 추출`
              : "다이어그램 없음"}
        </span>
        <div className="flex-1" />
        <button
          type="button"
          onClick={() => setLocalNonce((n) => n + 1)}
          title="다시 스캔"
          className="grid h-7 w-7 place-items-center rounded-md text-ink-soft transition-colors hover:bg-subtle hover:text-ink"
        >
          <RefreshCw size={14} />
        </button>
      </div>

      {/* gallery body */}
      {items !== null && items.length === 0 ? (
        <div className="flex flex-1 flex-col items-center justify-center px-6 text-center text-ink-soft">
          <div className="mb-4 grid h-16 w-16 place-items-center rounded-[18px] bg-subtle text-ink-faint">
            <Network size={30} />
          </div>
          <div className="mb-1.5 font-serif text-[16px] font-semibold text-ink-muted">
            다이어그램이 없습니다
          </div>
          <div className="max-w-[320px] text-[12.5px] leading-[1.5]">
            산출물 문서에 포함된 mermaid 다이어그램이 여기에 모입니다.
          </div>
        </div>
      ) : (
        <div className="min-h-0 flex-1 overflow-auto">
          <div className="grid grid-cols-[repeat(auto-fill,minmax(260px,1fr))] gap-3 p-4">
            {items?.map((item) => (
              <button
                key={item.key}
                type="button"
                onClick={() => openZoom(item)}
                title="클릭하여 확대"
                className="flex cursor-zoom-in flex-col gap-2 rounded-lg border border-line bg-panel p-3 text-left shadow-xs transition-colors hover:border-line-strong"
              >
                <span className="flex items-center gap-1.5">
                  <span className="min-w-0 flex-1 truncate font-mono text-[11px] text-ink-soft">
                    {item.label}
                  </span>
                  {item.extra > 0 && (
                    <span className="shrink-0 rounded-full bg-subtle px-1.5 py-px text-[10px] text-ink-soft">
                      외 {item.extra}곳
                    </span>
                  )}
                </span>
                <span className="ow-diagram-figure block max-h-[240px] overflow-hidden">
                  <MermaidDiagram code={item.code} />
                </span>
              </button>
            ))}
          </div>
        </div>
      )}

      {/* enlarge modal (backdrop click / X / Escape to close): fills nearly the
          whole window instead of hugging the diagram, with zoom controls. */}
      {zoom && (
        <div
          className="fixed inset-0 z-50 flex bg-ink/40 p-4"
          onClick={() => setZoom(null)}
        >
          <div
            className="flex h-full w-full flex-col overflow-hidden rounded-xl border border-line-strong bg-panel shadow-lg"
            onClick={(e) => e.stopPropagation()}
          >
            {/* modal bar: source label + zoom controls + close */}
            <div className="flex h-10 shrink-0 items-center gap-2 border-b border-line bg-panel px-3">
              <span className="min-w-0 flex-1 truncate font-mono text-[12px] text-ink-soft">
                {zoom.label}
              </span>
              <div className="flex shrink-0 items-center overflow-hidden rounded-md border border-line">
                <button
                  type="button"
                  onClick={() => zoomBy(-1)}
                  title="축소"
                  className="grid h-7 w-7 place-items-center text-ink-soft transition-colors hover:bg-subtle hover:text-ink"
                >
                  <ZoomOut size={14} />
                </button>
                <button
                  type="button"
                  onClick={() => setScale(1)}
                  title="원래 크기로"
                  className="h-7 w-[52px] border-x border-line text-center font-mono text-[11.5px] text-ink-muted transition-colors hover:bg-subtle"
                >
                  {Math.round(scale * 100)}%
                </button>
                <button
                  type="button"
                  onClick={() => zoomBy(1)}
                  title="확대"
                  className="grid h-7 w-7 place-items-center text-ink-soft transition-colors hover:bg-subtle hover:text-ink"
                >
                  <ZoomIn size={14} />
                </button>
              </div>
              <button
                type="button"
                onClick={() => setZoom(null)}
                title="닫기"
                className="grid h-7 w-7 shrink-0 place-items-center rounded-md text-ink-soft transition-colors hover:bg-subtle hover:text-ink"
              >
                <X size={15} />
              </button>
            </div>
            {/* diagram body: centered while it fits, scrollable once zoomed past */}
            <div className="flex min-h-0 flex-1 overflow-auto bg-app">
              <div className="m-auto p-6" style={{ zoom: scale }}>
                <MermaidDiagram code={zoom.code} />
              </div>
            </div>
          </div>
        </div>
      )}
    </div>
  );
}
