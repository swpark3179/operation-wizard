// Requirements form protocol (frontend-only — not a Rust serde mirror).
//
// The option catalog itself is now static per category (see `options.ts`) and
// shown on entry; this module hosts the shared pieces: the `ClarifyQuestion`/
// `ClarifyAnswer` shapes the form renders, `formatClarifyAnswers` (answers →
// wire + display), and the **prefill** protocol (`prefillInstruction` /
// `parsePrefill`) that auto-fills known options from the launcher prompt via a
// hidden agent turn (design D36). `parseClarify`/`stripClarifyBlock`/
// `CLARIFY_INSTRUCTION` remain as a reference implementation of the fenced-block
// pattern (agent GENERATES the questions) — the template to copy when forcing a
// new agent-emitted result format. There is no custom tool-call channel for
// local CLI agents, so every such protocol rides in fenced text blocks and any
// parse failure falls back to plain chat so the conversation never breaks.

export type ClarifyType = "single" | "multi" | "text" | "folder";

export interface ClarifyQuestion {
  id: string;
  label: string;
  /** "folder" renders a native folder-picker button (value = absolute path);
   * folder questions are excluded from the prefill protocol — an agent cannot
   * know local paths (D45). */
  type: ClarifyType;
  /** Choices for single/multi (required for those types). */
  options?: string[];
  required?: boolean;
}

export interface ClarifyAnswer {
  id: string;
  label: string;
  type: ClarifyType;
  /** string for single/text, string[] for multi. */
  value: string | string[];
}

/** Prepended (invisibly) to the user's first plan-turn prompt on the wire. */
export const CLARIFY_INSTRUCTION = `[시스템 지시: 요구사항 명확화 단계]
당신은 "개발 계획 수립" 작업을 맡았습니다. 사용자의 첫 요청에 곧바로 계획이나 코드를 작성하지 말고,
계획 수립에 꼭 필요한 핵심 정보를 파악하기 위한 "요구사항 명확화 질문"을 먼저 생성하세요.

반드시 아래 규칙을 지키세요:
1. 응답의 가장 처음에 아래 형식의 \`\`\`clarify 코드 블록 하나만 출력하고, 그 앞뒤에 다른 설명 텍스트를 쓰지 마세요.
2. 블록 안은 유효한 JSON이어야 하며 스키마는 다음과 같습니다:
   {
     "questions": [
       { "id": "겹치지_않는_짧은_영문_슬러그", "label": "사용자에게 보여줄 질문(한국어)",
         "type": "single" | "multi" | "text", "options": ["선택지1", "선택지2"], "required": true }
     ]
   }
   - "type"가 "single"이면 단일 선택(라디오), "multi"면 복수 선택(체크박스)이며 이때 "options"는 필수입니다.
   - "type"가 "text"이면 자유 입력이며 "options"는 생략합니다.
   - 질문은 계획 수립에 실제로 영향을 주는 것만 3~6개로 제한하세요.
3. 이 블록을 출력한 뒤에는 멈추고 사용자의 답변을 기다리세요. 계획/코드/추가 설명을 아직 작성하지 마세요.

예시:
\`\`\`clarify
{
  "questions": [
    { "id": "scope", "label": "이번 변경의 범위는 무엇인가요?", "type": "single", "options": ["버그 수정", "신규 기능", "리팩터링"], "required": true },
    { "id": "targets", "label": "영향을 받는 영역을 모두 선택하세요.", "type": "multi", "options": ["프론트엔드", "백엔드", "DB"] },
    { "id": "notes", "label": "그 밖에 알아야 할 제약이 있나요?", "type": "text" }
  ]
}
\`\`\`

사용자의 실제 요청:
---
`;

/** Shown in the chat when the clarify block is parsed out to the canvas form. */
export const CLARIFY_NOTE =
  "요구사항 확인 질문을 오른쪽 캔버스 '요구사항' 탭에 표시했습니다. 항목을 작성해 제출해 주세요.";

/** All fenced code blocks in a string, with their tag and inner body. */
function fencedBlocks(content: string): { tag: string; body: string }[] {
  const out: { tag: string; body: string }[] = [];
  const re = /```([^\n`]*)\n?([\s\S]*?)```/g;
  let m: RegExpExecArray | null;
  while ((m = re.exec(content)) !== null) {
    out.push({ tag: (m[1] ?? "").trim().toLowerCase(), body: m[2] ?? "" });
  }
  return out;
}

function coerceQuestions(parsed: unknown): ClarifyQuestion[] | null {
  const raw = (parsed as { questions?: unknown })?.questions;
  if (!Array.isArray(raw)) return null;
  const questions: ClarifyQuestion[] = [];
  for (const q of raw) {
    const o = q as Partial<ClarifyQuestion>;
    if (typeof o?.id !== "string" || typeof o?.label !== "string") continue;
    if (o.type !== "single" && o.type !== "multi" && o.type !== "text" && o.type !== "folder")
      continue;
    if (o.type === "single" || o.type === "multi") {
      const opts = Array.isArray(o.options)
        ? o.options.filter((x): x is string => typeof x === "string" && x.trim().length > 0)
        : [];
      if (opts.length === 0) continue; // choice question needs options
      questions.push({ id: o.id, label: o.label, type: o.type, options: opts, required: !!o.required });
    } else {
      questions.push({ id: o.id, label: o.label, type: o.type, required: !!o.required });
    }
  }
  return questions.length ? questions : null;
}

/** Extract clarify questions from an assistant message, or null (→ plain chat). */
export function parseClarify(content: string): ClarifyQuestion[] | null {
  const blocks = fencedBlocks(content);
  if (blocks.length === 0) return null;
  // Prefer the explicit ```clarify block, else any fenced JSON with `questions`.
  const ordered = [...blocks.filter((b) => b.tag === "clarify"), ...blocks];
  for (const b of ordered) {
    try {
      const parsed = JSON.parse(b.body.trim());
      const qs = coerceQuestions(parsed);
      if (qs) return qs;
    } catch {
      /* try next block */
    }
  }
  return null;
}

/** Remove the clarify/JSON fenced block(s) from the visible chat text. */
export function stripClarifyBlock(content: string): string {
  return content.replace(/```([^\n`]*)\n?[\s\S]*?```/g, (full, tag: string) => {
    const t = (tag ?? "").trim().toLowerCase();
    if (t === "clarify" || t === "json" || t === "") return "";
    return full; // keep unrelated code blocks
  });
}

/** Format submitted answers into a wire prompt (for the agent) + a compact
 * display bubble (for the chat). */
export function formatClarifyAnswers(answers: ClarifyAnswer[]): { wire: string; display: string } {
  const asText = (a: ClarifyAnswer) =>
    Array.isArray(a.value) ? a.value.join(", ") : a.value;

  const wireLines = answers.map((a, i) => `${i + 1}. ${a.label}\n   → ${asText(a) || "(미응답)"}`);
  const wire =
    "아래는 요구사항 명확화 질문에 대한 답변입니다.\n\n" +
    wireLines.join("\n") +
    "\n\n이 답변을 반영해 작업을 계속 진행해 주세요.";

  const clip = (s: string) => (s.length > 40 ? s.slice(0, 40) + "…" : s);
  const displayLines = answers.map((a) => `· ${a.label}: ${clip(asText(a) || "(미응답)")}`);
  const display = "요구사항 답변을 제출했습니다.\n" + displayLines.join("\n");

  return { wire, display };
}

// ── Prefill (auto-fill) — the inverse of clarify: the agent ANSWERS a fixed
// option catalog from the user's launcher prompt, filling only what it can
// confidently infer (design D36). A hidden turn runs this; parsePrefill lifts
// the answers into the form. Anything uncertain is left for the user. ──────────

/** One line describing a question for the prefill instruction. */
function serializeQuestion(q: ClarifyQuestion): string {
  const req = q.required ? ", 필수" : "";
  const opts =
    (q.type === "single" || q.type === "multi") && q.options?.length
      ? ` | 옵션: ${q.options.join(" / ")}`
      : "";
  return `- id "${q.id}" (${q.type}${req}): ${q.label}${opts}`;
}

/** Build the instruction that asks the agent to prefill known option answers
 * from the user's request. Prepended (invisibly) to a hidden prefill turn.
 * Folder questions are excluded — an agent cannot know local paths. */
export function prefillInstruction(questions: ClarifyQuestion[], userPrompt: string): string {
  questions = questions.filter((q) => q.type !== "folder");
  return `[시스템 지시: 요청 자동 분석 단계]
아래는 사용자가 답해야 할 선택 항목 목록입니다. 사용자의 요청을 읽고, 요청에서 "확실하게" 알 수 있는 항목만 골라 답을 채우세요.

반드시 아래 규칙을 지키세요:
1. 확신할 수 없는 항목은 절대 추측하지 말고 생략하세요(그 항목은 사용자가 직접 고릅니다).
2. 응답의 가장 처음에 아래 형식의 \`\`\`prefill 코드 블록 하나만 출력하고, 그 앞뒤에 다른 텍스트를 쓰지 마세요.
3. 블록 안은 유효한 JSON이며 스키마는 { "answers": { "<질문id>": <답> } } 입니다.
   - "single" 항목의 답은 옵션 중 정확히 하나의 문자열입니다.
   - "multi" 항목의 답은 옵션들의 부분집합 배열입니다.
   - "text" 항목의 답은 문자열입니다.
4. 채울 항목이 하나도 없으면 { "answers": {} } 를 출력하세요. 이 턴에서는 계획/코드/설명을 작성하지 마세요.

선택 항목:
${questions.map(serializeQuestion).join("\n")}

사용자의 요청:
---
${userPrompt}
`;
}

/** Parse a ```prefill block into validated per-question answers (values coerced
 * to each question's type/options; unknown ids and invalid values dropped).
 * Returns {} on any failure → an empty form (never breaks the flow). */
export function parsePrefill(
  content: string,
  questions: ClarifyQuestion[],
): Record<string, string | string[]> {
  const byId = new Map(questions.map((q) => [q.id, q]));
  const blocks = fencedBlocks(content);
  const ordered = [...blocks.filter((b) => b.tag === "prefill"), ...blocks];
  for (const b of ordered) {
    let answers: unknown;
    try {
      answers = (JSON.parse(b.body.trim()) as { answers?: unknown })?.answers;
    } catch {
      continue;
    }
    if (!answers || typeof answers !== "object") continue;
    const out: Record<string, string | string[]> = {};
    for (const [id, raw] of Object.entries(answers as Record<string, unknown>)) {
      const q = byId.get(id);
      if (!q) continue;
      if (q.type === "folder") continue; // never agent-fillable (defensive)
      if (q.type === "text") {
        if (typeof raw === "string" && raw.trim()) out[id] = raw;
      } else if (q.type === "single") {
        if (typeof raw === "string" && q.options?.includes(raw)) out[id] = raw;
      } else {
        // multi
        if (Array.isArray(raw)) {
          const vals = raw.filter((x): x is string => typeof x === "string" && !!q.options?.includes(x));
          if (vals.length) out[id] = vals;
        }
      }
    }
    return out; // first block that parsed wins (even if empty answers)
  }
  return {};
}
