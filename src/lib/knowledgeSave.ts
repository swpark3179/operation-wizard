// Knowledge-save helpers (D59): turn a completed workflow's artifacts into an
// artifact-kind knowledge entry — an agent-generated summary (body) plus the
// document files copied into the knowledge store (save_knowledge_files).
//
// The summary turn runs ISOLATED (prefill pattern, D30 family): a one-shot
// runAgent call with no session id / resume, its own Channel, and no touch of
// ChatPanel's step cursor or persisted transcript. That makes it work
// identically for the completion banner, the manual 산출물-tab save, and
// loaded sessions — and it can run while the user keeps chatting.

import { Channel, cancelRun, runAgent, saveKnowledgeFiles } from "./api";
import { fencedBlocks } from "./clarify";
import type { KnowledgeEntry, RunEvent } from "./types";

/** Injection-time per-entry clip is 4000 chars (foundation.ts) — no point
 * storing a summary longer than what will ever be injected. */
const SUMMARY_CLIP = 4000;

/** The summary turn's output contract: exactly one ```summary fence. */
export function summaryInstruction(request: string, files: string[]): string {
  const fileList = files.map((f) => `- ${f}`).join("\n");
  return [
    "당신은 방금 완료된 작업을 이후 작업에서 재사용할 수 있는 지식으로 요약하는 역할입니다.",
    `다음 산출물 문서를 직접 읽고 내용을 파악하세요(절대경로):\n${fileList}`,
    request.trim() ? `원래 요청:\n${request.trim()}` : "",
    [
      "아래 항목을 담아 500~1000자의 한국어 요약을 작성하세요:",
      "1. 무엇을 요청받아 어떤 작업을 했는지",
      "2. 어떤 접근·방법을 사용했는지",
      "3. 각 산출물 문서에 무엇이 담겨 있는지",
      "4. 이후 유사 작업에서 참고할 점(주의사항·전제 포함)",
    ].join("\n"),
    "응답은 다른 텍스트 없이 ```summary 코드 펜스 하나로만 출력하세요:\n```summary\n(요약 본문)\n```",
  ]
    .filter(Boolean)
    .join("\n\n");
}

/** Extract the summary from the turn's output: the ```summary fence when the
 * agent followed the contract, else the whole reply (plain-text fallback —
 * this is what keeps plain agents' raw stdout usable). Null when empty. */
export function parseSummary(content: string): string | null {
  const fenced = fencedBlocks(content).find((b) => b.tag === "summary");
  const text = (fenced?.body ?? content).trim();
  return text ? text.slice(0, SUMMARY_CLIP) : null;
}

/** Run the isolated summary turn. `onText` streams the accumulating raw reply
 * (for live feedback); the promise resolves with the parsed summary, or null
 * on failure/cancel/empty (the caller falls back to manual editing — saving is
 * never blocked on the summary). */
export function generateKnowledgeSummary(args: {
  agentId: string;
  model: string | null;
  /** The project workdir — the artifact files live under it. */
  cwd: string;
  /** Absolute paths of the documents to summarize. */
  files: string[];
  /** The original request (seed prompt / project title) for context. */
  request: string;
  onText?: (accumulated: string) => void;
}): { promise: Promise<string | null>; cancel: () => void } {
  let runId: string | null = null;
  let canceled = false;
  let acc = "";

  const promise = new Promise<string | null>((resolve) => {
    const channel = new Channel<RunEvent>();
    channel.onmessage = (ev) => {
      switch (ev.type) {
        case "textDelta":
          acc += ev.delta;
          args.onText?.(acc);
          break;
        case "stdout":
          acc += ev.chunk;
          args.onText?.(acc);
          break;
        case "end":
          resolve(ev.status === "succeeded" && !canceled ? parseSummary(acc) : null);
          break;
        default:
          break;
      }
    };
    runAgent(
      {
        agentId: args.agentId,
        prompt: summaryInstruction(args.request, args.files),
        cwd: args.cwd,
        model: args.model,
        sessionId: null,
        resume: false,
        extraDirs: [], // the files are under cwd
      },
      channel,
    )
      .then((id) => {
        runId = id;
        if (canceled) void cancelRun(id).catch(() => {});
      })
      .catch(() => resolve(null)); // spawn failure → manual editing fallback
  });

  return {
    promise,
    cancel: () => {
      canceled = true;
      if (runId) void cancelRun(runId).catch(() => {});
    },
  };
}

/** Build the artifact-kind entry and hand copy + upsert to the backend
 * (`save_knowledge_files` — staged swap, D59). Shared by the completion-banner
 * panel and the manual 산출물-tab save. */
export function saveArtifactKnowledge(args: {
  entryId: string;
  title: string;
  summary: string;
  /** Absolute source paths (workdir-joined artifact files). */
  files: string[];
  source: { projectId: string; category: string; title: string };
}): Promise<KnowledgeEntry> {
  const entry: KnowledgeEntry = {
    id: args.entryId,
    title: args.title,
    body: args.summary,
    kind: "artifact",
    files: [], // set by the backend from the copied names
    sourceProjectId: args.source.projectId,
    sourceCategory: args.source.category,
    sourceTitle: args.source.title,
    createdAt: 0, // preserved server-side on re-save (upsert)
    updatedAt: 0,
  };
  return saveKnowledgeFiles(entry, args.files);
}
