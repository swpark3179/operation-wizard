// Shared types + constants for the operations workspace (Home → Workspace).

import type { ComponentType } from "react";
import { ClipboardList, BookOpen, Database, ShieldCheck } from "lucide-react";

/** The four work categories from the design. Only "plan" is fully specced; the
 * others route into the same generic chat+agent workspace for now. */
export type Category = "plan" | "guide" | "query" | "change";

export interface CategoryDef {
  id: Category;
  label: string;
  desc: string;
  icon: ComponentType<{ size?: number }>;
  /** Tailwind classes for the icon tile (semantic tokens only). */
  tile: string;
}

export const CATEGORIES: CategoryDef[] = [
  {
    id: "plan",
    label: "개발 계획 수립",
    desc: "요구사항 → 저장소 분석 → 영향도 → 계획서·변경 가이드",
    icon: ClipboardList,
    tile: "bg-accent-tint text-accent",
  },
  {
    id: "guide",
    label: "운영 가이드 생성",
    desc: "반복 운영 절차를 단계별 가이드 문서로 정리",
    icon: BookOpen,
    tile: "bg-info-bg text-info",
  },
  {
    id: "query",
    label: "데이터 조회",
    desc: "안전한 조회 쿼리 설계와 결과 정리·검증",
    icon: Database,
    tile: "bg-ok-bg text-ok",
  },
  {
    id: "change",
    label: "데이터 변경·권한",
    desc: "수정·스키마 변경·권한 부여 절차와 승인 흐름",
    icon: ShieldCheck,
    tile: "bg-warn-bg text-warn",
  },
];

export function categoryLabel(id: Category): string {
  return CATEGORIES.find((c) => c.id === id)?.label ?? "";
}

/** Compact relative time for the session history list (KO). */
export function sessionTime(ms: number): string {
  if (!ms) return "";
  const diff = Date.now() - ms;
  const min = Math.floor(diff / 60_000);
  if (min < 1) return "방금";
  if (min < 60) return `${min}분 전`;
  const hr = Math.floor(min / 60);
  if (hr < 24) return `${hr}시간 전`;
  const day = Math.floor(hr / 24);
  if (day < 7) return `${day}일 전`;
  return new Date(ms).toLocaleDateString();
}

/** Map a known agent-error signature to a friendly Korean hint, or null.
 *
 * The codex "Reconnecting… (invalid peer certificate: BadSignature)" text is
 * emitted by the codex CLI itself (a rustls TLS failure), not this app — it is
 * the classic signature of a corporate TLS-inspecting proxy whose re-signed
 * certificate codex does not trust. The app can only surface a clearer hint and
 * let the user recover with a new session. */
export function errorHint(msg: string | undefined): string | null {
  if (!msg) return null;
  const m = msg.toLowerCase();
  if (
    m.includes("peer certificate") ||
    m.includes("badsignature") ||
    m.includes("certificate") ||
    m.includes("tls")
  ) {
    return "사내망 TLS 인증서를 에이전트가 신뢰하지 못했습니다(프록시 재서명 인증서 추정). 새 세션으로 다시 시도하거나, 사내 CA/프록시 설정을 확인하세요.";
  }
  if (m.includes("reconnect") || m.includes("stream disconnected")) {
    return "스트림이 완료 전에 끊겼습니다(네트워크/프록시 문제 가능). 새 세션으로 다시 시도해 보세요.";
  }
  // codex 엔터프라이즈 관리 정책(Group requirements)이 샌드박스 모드를 제한할 때
  // codex CLI가 config 로드 단계에서 내는 오류. 앱은 정책 상한을 우회할 수 없다(D80).
  if (
    m.includes("allowed_sandbox_modes") ||
    m.includes("permissionprofile") ||
    m.includes("enterprise-managed") ||
    (m.includes("sandbox") && m.includes("not in the allowed set"))
  ) {
    return "사내 관리 정책이 codex 샌드박스를 제한하고 있습니다. 관리자(IT)에게 codex 정책의 allowed_sandbox_modes에 'read-only'가 포함되도록(예: [\"read-only\",\"workspace-write\"]) 정정을 요청하세요. 앱은 정책 상한을 우회할 수 없습니다. 우회가 필요하면 다른 에이전트(Claude Code·Gemini·Fabrix·AI Pro)로 진행할 수 있습니다.";
  }
  return null;
}

/** One conversation message rendered in the chat panel. */
export interface ChatMessage {
  role: "user" | "assistant";
  /** Streamed assistant text (or the user's prompt). */
  content: string;
  /** Streamed reasoning, if any. */
  thinking: string;
  /** Tool calls / results / usage timeline for an assistant turn. */
  events: TimelineEvent[];
  /** Set on a failed turn. */
  error?: string;
  /** True while this assistant message is still streaming. */
  streaming?: boolean;
  /** Workflow step-progress note (rendered as a centered subtle line, not a bubble). */
  system?: boolean;
}

/** A non-text entry in an assistant message's timeline. */
export type TimelineEvent =
  | { kind: "toolUse"; id: string; name: string; input: unknown }
  | { kind: "toolResult"; toolUseId: string; content: string; isError: boolean }
  | { kind: "usage"; inputTokens?: number; outputTokens?: number };
