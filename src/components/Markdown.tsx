// Markdown preview for the canvas file viewer (design D42): react-markdown +
// remark-gfm (tables), with ```mermaid fences rendered as real diagrams. All
// libraries are bundled (no CDN — works offline / on the corporate network);
// mermaid (~1.5 MB) is dynamically imported so Vite code-splits it and it only
// loads when the first diagram appears.

import { useEffect, useState } from "react";
import ReactMarkdown from "react-markdown";
import remarkGfm from "remark-gfm";

type MermaidModule = typeof import("mermaid").default;

let mermaidLoader: Promise<MermaidModule> | null = null;
let renderSeq = 0;

function loadMermaid(): Promise<MermaidModule> {
  if (!mermaidLoader) {
    mermaidLoader = import("mermaid").then((m) => {
      // `strict` disables scripts/click handlers inside diagrams; mermaid
      // sanitizes the produced SVG itself (bundled DOMPurify).
      m.default.initialize({ startOnLoad: false, securityLevel: "strict", theme: "neutral" });
      return m.default;
    });
  }
  return mermaidLoader;
}

/** One ```mermaid fence → SVG. A render failure falls back to the raw code
 * plus a small note, so the viewer never breaks on a malformed diagram. */
function MermaidDiagram({ code }: { code: string }) {
  const [svg, setSvg] = useState<string | null>(null);
  const [error, setError] = useState<string | null>(null);

  useEffect(() => {
    let cancelled = false;
    setSvg(null);
    setError(null);
    const id = `ow-mermaid-${++renderSeq}`; // unique per render attempt
    loadMermaid()
      .then((mermaid) => mermaid.render(id, code))
      .then((res) => {
        if (!cancelled) setSvg(res.svg);
      })
      .catch((e) => {
        if (!cancelled) setError(String(e));
      });
    return () => {
      cancelled = true;
    };
  }, [code]);

  if (error) {
    return (
      <span className="ow-mermaid block">
        <code className="block whitespace-pre-wrap font-mono text-[12px] leading-[1.6]">{code}</code>
        <span className="mt-1 block text-[11.5px] text-bad">다이어그램 렌더 실패: {error}</span>
      </span>
    );
  }
  if (svg === null) {
    return <span className="ow-mermaid block text-[12px] text-ink-faint">다이어그램 렌더 중…</span>;
  }
  // Safe: mermaid sanitizes its own SVG output (securityLevel "strict").
  return <span className="ow-mermaid block" dangerouslySetInnerHTML={{ __html: svg }} />;
}

/** Rendered markdown document (styling from `.markdown-body` in global.css). */
export function MarkdownView({ content }: { content: string }) {
  return (
    <div className="markdown-body">
      <ReactMarkdown
        remarkPlugins={[remarkGfm]}
        components={{
          code({ className, children, ...props }) {
            const lang = /language-(\w+)/.exec(className ?? "")?.[1];
            if (lang === "mermaid") {
              return <MermaidDiagram code={String(children).trim()} />;
            }
            return (
              <code className={className} {...props}>
                {children}
              </code>
            );
          },
        }}
      >
        {content}
      </ReactMarkdown>
    </div>
  );
}
