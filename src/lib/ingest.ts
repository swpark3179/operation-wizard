// Confluence ingest progress store (D51) — a module-level singleton that lives
// OUTSIDE the React tree. The backend crawl already runs on a detached worker
// thread (confluence.rs) and survives view switches; what did NOT survive was
// the receiving Channel and the progress tallies, which used to be local state
// of the Knowledge view and were destroyed on unmount. This store owns both,
// so the user can start an ingest, keep chatting elsewhere, and come back to a
// live progress view. Scope: app-process lifetime (an app restart also kills
// the backend worker — nothing to restore).

import { useSyncExternalStore } from "react";
import { Channel, cancelIngest, startConfluenceIngest } from "./api";
import type { IngestEvent } from "./types";

export interface IngestProgress {
  fetched: number;
  ingested: number;
  failed: number;
  /** Last page title (or a status line) for the live ticker. */
  last: string;
}

export interface IngestState {
  status: "idle" | "running" | "done";
  ingestId: string | null;
  progress: IngestProgress | null;
  /** First few per-page failures + fatal errors (bounded, like the old view). */
  failures: string[];
  /** One-line outcome once the run ends (or the start error). */
  summary: string | null;
}

const IDLE: IngestState = {
  status: "idle",
  ingestId: null,
  progress: null,
  failures: [],
  summary: null,
};

let state: IngestState = IDLE;
const listeners = new Set<() => void>();

function patch(p: Partial<IngestState>) {
  state = { ...state, ...p };
  listeners.forEach((l) => l());
}

export function subscribeIngest(listener: () => void): () => void {
  listeners.add(listener);
  return () => {
    listeners.delete(listener);
  };
}

export function getIngestState(): IngestState {
  return state;
}

/** Subscribe a component to the ingest state (re-renders per event). */
export function useIngestState(): IngestState {
  return useSyncExternalStore(subscribeIngest, getIngestState);
}

/** Start a crawl+ingest run. No-op while one is already running. */
export async function startIngest(): Promise<void> {
  if (state.status === "running") return;
  patch({
    status: "running",
    ingestId: null,
    progress: { fetched: 0, ingested: 0, failed: 0, last: "시작 중…" },
    failures: [],
    summary: null,
  });

  const channel = new Channel<IngestEvent>();
  channel.onmessage = (ev) => {
    const p = state.progress ?? { fetched: 0, ingested: 0, failed: 0, last: "" };
    switch (ev.type) {
      case "started":
        patch({ progress: { ...p, last: `수집 시작 (${ev.rootId})` } });
        break;
      case "pageFetched":
        patch({ progress: { ...p, fetched: ev.fetched, last: ev.title } });
        break;
      case "pageIngested":
        patch({ progress: { ...p, ingested: ev.ingested, last: ev.title } });
        break;
      case "pageFailed":
        patch({
          progress: { ...p, failed: p.failed + 1, last: ev.title },
          failures:
            state.failures.length < 5
              ? [...state.failures, `${ev.title || ev.pageId}: ${ev.message}`]
              : state.failures,
        });
        break;
      case "error":
        patch({ failures: [...state.failures, ev.message] });
        break;
      case "end": {
        const label =
          ev.status === "succeeded" ? "완료" : ev.status === "canceled" ? "중지됨" : "실패";
        patch({
          status: "done",
          ingestId: null,
          summary: `${label} — 임베딩 ${ev.ingested}건 · 실패 ${ev.failed}건`,
        });
        break;
      }
    }
  };

  try {
    const id = await startConfluenceIngest(channel);
    // The end event may already have arrived for a very short run — don't
    // resurrect a finished state with a stale id. (Read via the accessor:
    // TS would otherwise keep the early-guard narrowing of `state`.)
    if (getIngestState().status === "running") patch({ ingestId: id });
  } catch (e) {
    patch({ status: "done", ingestId: null, progress: null, summary: String(e) });
  }
}

/** Request cancellation of the running ingest (the worker checks the flag
 * between HTTP calls; the `end{canceled}` event closes the run). */
export function stopIngest(): void {
  if (state.ingestId) void cancelIngest(state.ingestId).catch(() => {});
}
