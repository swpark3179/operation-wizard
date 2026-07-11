// Workflow artifact helpers (design D58). The canvas 산출물/다이어그램 tabs
// derive their content from the category's runtime workflow: every step that
// writes a `file` is one artifact. Pure functions only (no React) — consumed by
// WorkspaceView (routing), ArtifactsPanel (list + existence probe) and
// DiagramGallery (mermaid extraction).

import type { Category } from "../components/workspace";
import type { Settings } from "./types";
import { runtimeWorkflowFor } from "./workflow";

/** One workflow-produced document: a runtime step that writes a file. The
 * stepId matches `StepProgress.id` (including synthetic `${id}-html` render
 * sub-steps), so live workflow status can be joined onto the artifact row. */
export interface ArtifactDef {
  stepId: string;
  /** Step display name, e.g. "계획 생성". */
  name: string;
  /** Workdir-relative output path, e.g. "docs/plan.md". */
  file: string;
}

/** Join a Windows workdir with a relative file path (for opening produced
 * files). Moved from ChatPanel so the artifact routing check and the open
 * call share one join. */
export function joinWorkdirPath(dir: string, rel: string): string {
  return `${dir.replace(/[\\/]+$/, "")}\\${rel.replace(/\//g, "\\")}`;
}

/** The category's artifacts: runtime steps that write a file, in step order. */
export function artifactsFor(category: Category, settings: Settings | null): ArtifactDef[] {
  return runtimeWorkflowFor(category, settings)
    .filter((s) => !!s.file?.trim())
    .map((s) => ({ stepId: s.id, name: s.name?.trim() || s.id, file: s.file!.trim() }));
}

/** Normalize a path for identity comparison on Windows: one separator style,
 * no trailing separator, case-insensitive. */
export function normalizePathKey(p: string): string {
  return p.replace(/\//g, "\\").replace(/\\+$/, "").toLowerCase();
}

/** Absolute parent directory of an artifact ("docs/plan.md" → `<workdir>\docs`)
 * — the folder the existence probe lists. */
export function artifactParentDir(workdir: string, file: string): string {
  const abs = joinWorkdirPath(workdir, file);
  const idx = abs.lastIndexOf("\\");
  return idx > 0 ? abs.slice(0, idx) : abs;
}

export function isMarkdownFile(path: string): boolean {
  return /\.(md|markdown)$/i.test(path);
}

/** Every ```mermaid fenced block in a markdown document. A line scanner that
 * tracks open/close fences (``` or ~~~, CommonMark rules: ≤3 leading spaces,
 * closing fence of the same char at least as long), so mermaid fences nested
 * inside other fences (e.g. a ````markdown example) are not extracted. */
export function extractMermaidBlocks(md: string): string[] {
  const out: string[] = [];
  let fence: { char: string; len: number; mermaid: boolean } | null = null;
  let buf: string[] = [];
  for (const line of md.split(/\r?\n/)) {
    const m = /^ {0,3}(`{3,}|~{3,})(.*)$/.exec(line);
    if (fence) {
      if (m && m[1][0] === fence.char && m[1].length >= fence.len && m[2].trim() === "") {
        if (fence.mermaid) out.push(buf.join("\n"));
        fence = null;
        buf = [];
      } else if (fence.mermaid) {
        buf.push(line);
      }
      continue;
    }
    if (m) {
      const info = m[2].trim();
      // A backtick fence's info string cannot contain backticks (CommonMark).
      if (m[1][0] === "`" && info.includes("`")) continue;
      fence = { char: m[1][0], len: m[1].length, mermaid: /^mermaid\b/i.test(info) };
    }
  }
  return out;
}
