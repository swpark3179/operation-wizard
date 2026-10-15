// Repository-map artifact (frontend-only, D94). The codebase step's preflight
// computes the app's non-AI repository map (codemap.rs `repo_map`, D88) for the
// agent's prompt; the same text is also saved next to the AI's analysis as a
// workflow artifact (산출물 hub, knowledge save, hand-off index).
//
// The map is written for prompts — plain lines with indentation (directory
// tree, dependency chains, outlines) and tool hints — which a markdown preview
// would fold into paragraphs. `repoMapDocument` keeps the headings and puts each
// section body in a monospace block so the layout survives.

/** Workdir-relative path of the saved map (one codebase step per workflow). */
export const REPO_MAP_FILE = "docs/repo-map.md";

/** Artifact display name. */
export const REPO_MAP_NAME = "저장소 지도 (비AI 정적 분석)";

/** Artifact id of a codebase step's map. It matches no workflow step, so the
 * 산출물 row status comes from the file's existence. */
export function repoMapArtifactId(stepId: string): string {
  return `${stepId}-map`;
}

/** Prompt-only lines/phrases dropped from the document (tool hints of the
 * outline renderer; the "already attached" note of the SQL-body heading). */
const DROP_LINE = /^본문이 필요하면 read_file\(/;
const HEADING_NOTE = /\s*\(발췌 — 이미 첨부되었으니 다시 읽지 않아도 됨\)/;

function codeSpan(s: string): string {
  return s.includes("`") ? `\`\` ${s} \`\`` : `\`${s}\``;
}

/** Longest backtick run in the lines (so the wrapping fence is longer). */
function longestBackticks(lines: string[]): number {
  let max = 0;
  for (const l of lines) for (const m of l.matchAll(/`+/g)) max = Math.max(max, m[0].length);
  return max;
}

function stamp(d: Date): string {
  const p = (n: number) => String(n).padStart(2, "0");
  return `${d.getFullYear()}-${p(d.getMonth() + 1)}-${p(d.getDate())} ${p(d.getHours())}:${p(d.getMinutes())}`;
}

/** The markdown document saved as `REPO_MAP_FILE`: a header (what this is,
 * codebase, time, request keywords) + the map's sections with their bodies in
 * monospace blocks. Fenced blocks the map already has (SQL bodies) are kept. */
export function repoMapDocument(
  map: string,
  meta: { codebase: string; terms: string[]; extended: boolean; generatedAt: Date },
): string {
  const lines = map.split(/\r?\n/);
  const first = lines.findIndex((l) => /^##\s/.test(l));
  // Preamble = the map's own H1 (replaced by ours) + its stats line(s).
  const preamble = (first < 0 ? lines : lines.slice(0, first)).filter((l) => l.trim() && !/^#\s/.test(l));
  const body = first < 0 ? [] : lines.slice(first);

  const out: string[] = [
    `# ${REPO_MAP_NAME}`,
    "",
    "> 앱이 AI 없이 정적 분석(휴리스틱 파서)으로 만든 코드베이스 지도입니다. 근사치이므로 핵심 경로는 실제 코드로 확인하세요.",
    "> 코드베이스 분석 단계의 AI 분석서는 이 지도를 출발점으로 작성됩니다.",
    "",
    `- 코드베이스: ${codeSpan(meta.codebase)}`,
    `- 생성: ${stamp(meta.generatedAt)} · 상세 수준: ${meta.extended ? "확장(아웃라인·요청 SQL 본문 추가)" : "표준"}`,
    `- 요청 키워드: ${meta.terms.length ? meta.terms.map(codeSpan).join(", ") : "없음"}`,
    ...preamble.map((l) => `- ${l.trim()}`),
    "",
  ];

  let group: string[] = [];
  const flush = () => {
    while (group.length && !group[0].trim()) group.shift();
    while (group.length && !group[group.length - 1].trim()) group.pop();
    if (group.length) {
      const fence = "`".repeat(Math.max(3, longestBackticks(group) + 1));
      out.push(`${fence}text`, ...group, fence, "");
    }
    group = [];
  };
  let open: { char: string; len: number } | null = null;
  for (const raw of body) {
    const line = raw.replace(/\s+$/, "");
    if (open) {
      out.push(line);
      const f = /^ {0,3}(`{3,}|~{3,})\s*$/.exec(line);
      if (f && f[1][0] === open.char && f[1].length >= open.len) {
        open = null;
        out.push("");
      }
      continue;
    }
    const f = /^ {0,3}(`{3,}|~{3,})(.*)$/.exec(line);
    if (f && !(f[1][0] === "`" && f[2].includes("`"))) {
      flush();
      open = { char: f[1][0], len: f[1].length };
      out.push(line);
      continue;
    }
    if (/^#{2,6}\s/.test(line)) {
      flush();
      out.push(line.replace(HEADING_NOTE, " (발췌)"), "");
      continue;
    }
    if (DROP_LINE.test(line)) continue;
    group.push(line);
  }
  flush();
  if (open) out.push(open.char.repeat(open.len)); // unbalanced input — close it
  return `${out.join("\n").replace(/\n{3,}/g, "\n\n").trimEnd()}\n`;
}
