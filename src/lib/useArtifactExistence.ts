// Shared artifact existence probe (D58 → extracted for D59): which of a
// workflow's artifact files are actually on disk. Lists the (few, unique)
// parent folders with listDir and collects file paths — a missing folder just
// means "nothing there yet". Used by ArtifactsPanel (status chips) and
// KnowledgeSavePanel (checkbox gating).

import { useEffect, useState } from "react";
import { listDir } from "./api";
import { artifactParentDir, normalizePathKey, type ArtifactDef } from "./artifacts";

/** Normalized absolute paths of files that exist under the artifacts' parent
 * dirs, or null while the first probe is in flight. Re-probes on
 * `refreshNonce` (a workflow step wrote a file). */
export function useArtifactExistence(
  workdir: string | null,
  artifacts: ArtifactDef[],
  refreshNonce?: number,
): Set<string> | null {
  const [existing, setExisting] = useState<Set<string> | null>(null);

  useEffect(() => {
    if (!workdir) {
      setExisting(null);
      return;
    }
    let cancelled = false;
    const dirs = [...new Set(artifacts.map((a) => artifactParentDir(workdir, a.file)))];
    void Promise.allSettled(dirs.map((d) => listDir(d))).then((results) => {
      if (cancelled) return;
      const set = new Set<string>();
      for (const r of results) {
        if (r.status !== "fulfilled") continue;
        for (const e of r.value) if (!e.isDir) set.add(normalizePathKey(e.path));
      }
      setExisting(set);
    });
    return () => {
      cancelled = true;
    };
  }, [workdir, artifacts, refreshNonce]);

  return existing;
}
