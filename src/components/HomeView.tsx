import { useEffect, useState } from "react";
import { Sparkles, ArrowUp, AlertTriangle, ClipboardList, FolderOpen, Loader2, X } from "lucide-react";
import { CATEGORIES, sessionTime, type Category } from "./workspace";
import { listProjects, loadSession, pickFolder } from "../lib/api";
import { useAutoGrow } from "../lib/useAutoGrow";
import type { AgentInfo, DetectedAgent, ProjectSummary, StoredSession } from "../lib/types";

/** Last path segment of a Windows/Unix path, for the folder chip. */
function basename(p: string): string {
  const parts = p.split(/[\\/]/).filter(Boolean);
  return parts[parts.length - 1] ?? p;
}

export function HomeView({
  agents,
  detected,
  onOpenAgents,
  onStart,
  onOpenSession,
}: {
  /** Detection registry/results for the no-agent onboarding banner (D57). */
  agents: AgentInfo[];
  detected: Record<string, DetectedAgent>;
  /** Navigate to the Agents view (undetected-agent onboarding, D57). */
  onOpenAgents?: () => void;
  /** Start a new project. `workdir` = a folder chosen on Home, or undefined → auto. */
  onStart: (category: Category, prompt: string, workdir?: string) => void;
  /** Open a recent project's latest session (adopts its id + resolved workdir
   * + stored codebase path). */
  onOpenSession: (
    session: StoredSession,
    projectId: string,
    workdir: string,
    codebasePath?: string | null,
  ) => void;
}) {
  const [prompt, setPrompt] = useState("");
  const promptRef = useAutoGrow(prompt, 200);
  const [projects, setProjects] = useState<ProjectSummary[]>([]);
  // Optional folder for the project about to start (transient; reset each Home
  // visit). Unset → the new project runs in its own auto-created folder.
  const [chosenFolder, setChosenFolder] = useState<string | null>(null);
  // Failures that used to be silently swallowed (project open, folder picker)
  // now surface here (D57).
  const [uiError, setUiError] = useState<string | null>(null);

  // Agent detection rollup (D57): still detecting → subtle hint; every agent
  // resolved but none available → onboarding banner (sending would only fail).
  const detecting = agents.length === 0 || agents.some((a) => !detected[a.id]);
  const noneAvailable =
    !detecting && agents.length > 0 && agents.every((a) => !detected[a.id]?.available);

  const send = () => onStart("plan", prompt.trim(), chosenFolder ?? undefined);

  const chooseFolder = async () => {
    try {
      const dir = await pickFolder();
      if (dir) setChosenFolder(dir);
    } catch (e) {
      setUiError(`폴더 선택에 실패했습니다 — ${String(e)}`);
    }
  };

  // All saved projects, newest activity first. Hide empty projects (a folder can
  // exist briefly before its first session is saved).
  useEffect(() => {
    listProjects()
      .then((ps) => setProjects(ps.filter((p) => p.sessionCount > 0)))
      .catch(() => setProjects([]));
  }, []);

  // Open a project: open its latest session (the workspace adopts the project's
  // id + resolved workdir), or start a fresh chat if it has none. A load
  // failure stays on Home with a visible message instead of silently dropping
  // into an unrelated empty chat (D57).
  const openProject = async (p: ProjectSummary) => {
    setUiError(null);
    try {
      if (p.lastSessionId) {
        onOpenSession(await loadSession(p.id, p.lastSessionId), p.id, p.workdir, p.codebasePath);
      } else {
        onStart("plan", "");
      }
    } catch (e) {
      setUiError(`프로젝트를 열지 못했습니다 — ${String(e)}`);
    }
  };

  return (
    <div className="h-full overflow-auto">
      <div className="mx-auto max-w-[820px] px-8 pb-14 pt-16">
        {/* hero */}
        <div className="mb-8 text-center">
          <div className="mb-4 inline-flex items-center gap-1.5 rounded-full bg-accent-tint px-3 py-1.5 text-[12px] font-semibold text-accent">
            <Sparkles size={14} />
            운영 업무 어시스턴트
          </div>
          <h1 className="mb-2 font-serif text-[30px] font-semibold tracking-[-0.02em] text-ink-strong">
            무엇을 도와드릴까요?
          </h1>
          <p className="text-[14px] text-ink-muted">
            업무 카테고리를 고르고 대화로 진행하세요. 첫 질문에서 요구사항을 함께 명확히 합니다.
          </p>
        </div>

        {/* no-agent onboarding (D57) */}
        {noneAvailable && (
          <div className="mb-4 flex items-center gap-2 rounded-[12px] border border-line bg-warn-bg px-3.5 py-2.5 text-[12.5px] text-warn">
            <AlertTriangle size={15} className="shrink-0" />
            <span className="min-w-0 flex-1">
              사용 가능한 CLI 에이전트가 없습니다. 대화를 시작해도 실행이 실패합니다 — 에이전트를
              설치하거나 실행 파일 경로를 설정해 주세요.
            </span>
            {onOpenAgents && (
              <button
                type="button"
                onClick={onOpenAgents}
                className="shrink-0 rounded-md border border-line bg-panel px-2.5 py-1 font-medium text-ink-muted transition-colors hover:bg-subtle"
              >
                Agents에서 설정
              </button>
            )}
          </div>
        )}
        {detecting && (
          <div className="mb-4 flex items-center justify-center gap-1.5 text-[12px] text-ink-soft">
            <Loader2 size={13} className="animate-spin" />
            로컬 에이전트 탐지 중…
          </div>
        )}
        {uiError && (
          <div className="mb-4 flex items-center gap-2 rounded-[12px] border border-bad-border bg-bad-bg px-3.5 py-2.5 text-[12.5px] text-bad">
            <AlertTriangle size={15} className="shrink-0" />
            <span className="min-w-0 flex-1" title={uiError}>
              {uiError}
            </span>
            <button
              type="button"
              onClick={() => setUiError(null)}
              aria-label="닫기"
              className="grid shrink-0 place-items-center rounded p-0.5 transition-opacity hover:opacity-70"
            >
              <X size={14} />
            </button>
          </div>
        )}

        {/* prompt composer */}
        <div className="mb-3 rounded-[16px] border border-line-strong bg-panel px-4 pb-3 pt-3.5 shadow-md">
          <textarea
            ref={promptRef}
            value={prompt}
            onChange={(e) => setPrompt(e.target.value)}
            onKeyDown={(e) => {
              // Enter = 전송, Shift+Enter / Ctrl+Enter = 줄바꿈(기본 동작 허용)
              if (e.key === "Enter" && !e.shiftKey && !e.ctrlKey && !e.metaKey) {
                e.preventDefault();
                send();
              }
            }}
            rows={2}
            placeholder="예: 주문 목록 화면에 배송상태 필터를 추가하고, 행 클릭 시 배송 상세 팝업을 띄우고 싶어요…"
            className="max-h-[200px] min-h-[52px] w-full resize-none overflow-y-auto bg-transparent text-[14.5px] leading-[1.55] text-ink outline-none placeholder:text-ink-faint"
          />
          <div className="mt-1.5 flex items-center gap-2">
            <span className="inline-flex items-center gap-1.5 text-[12px] text-ink-soft">
              <ClipboardList size={14} />
              개발 계획 수립
            </span>
            <div className="flex-1" />
            <button
              type="button"
              onClick={send}
              title="전송"
              className="grid h-9 w-9 place-items-center rounded-[10px] bg-accent text-white shadow-xs transition-colors hover:bg-accent-strong"
            >
              <ArrowUp size={18} />
            </button>
          </div>
        </div>

        {/* working folder (per new project; unset → auto-created) */}
        <div className="mb-9 flex items-center gap-2">
          <button
            type="button"
            onClick={() => void chooseFolder()}
            title={chosenFolder ?? "작업 폴더 지정"}
            className="inline-flex max-w-[420px] items-center gap-1.5 rounded-lg border border-line bg-panel px-3 py-1.5 text-[12.5px] text-ink-muted transition-colors hover:bg-subtle"
          >
            <FolderOpen size={14} className="shrink-0 text-accent" />
            <span className="truncate">{chosenFolder ? basename(chosenFolder) : "작업 폴더 지정"}</span>
          </button>
          {chosenFolder ? (
            <button
              type="button"
              onClick={() => setChosenFolder(null)}
              title="폴더 지정 해제"
              className="grid h-6 w-6 place-items-center rounded-md text-ink-soft transition-colors hover:bg-subtle hover:text-ink"
            >
              <X size={13} />
            </button>
          ) : (
            <span className="text-[12px] text-ink-soft">미지정 시 새 프로젝트 폴더가 자동 생성됩니다</span>
          )}
        </div>

        {/* categories */}
        <div className="mb-3 text-[11px] font-bold uppercase tracking-[0.08em] text-ink-soft">
          업무 카테고리
        </div>
        <div className="mb-9 grid grid-cols-2 gap-3.5">
          {CATEGORIES.map(({ id, label, desc, icon: Icon, tile }) => (
            <button
              key={id}
              type="button"
              onClick={() => onStart(id, prompt.trim(), chosenFolder ?? undefined)}
              className="flex gap-3 rounded-[14px] border border-line bg-panel p-4 text-left shadow-sm transition-[transform,box-shadow] duration-[120ms] hover:-translate-y-0.5 hover:shadow-md"
            >
              <span className={`grid h-10 w-10 shrink-0 place-items-center rounded-[11px] ${tile}`}>
                <Icon size={21} />
              </span>
              <span className="min-w-0">
                <span className="block font-serif text-[16px] font-semibold text-ink-strong">
                  {label}
                </span>
                <span className="mt-0.5 block text-[12.5px] leading-[1.45] text-ink-muted">
                  {desc}
                </span>
              </span>
            </button>
          ))}
        </div>

        {/* recent — saved projects (working folders); open the latest session */}
        <div className="mb-2.5 text-[11px] font-bold uppercase tracking-[0.08em] text-ink-soft">
          최근 작업
        </div>
        {projects.length === 0 ? (
          <div className="rounded-[14px] border border-line bg-panel px-4 py-8 text-center text-[12.5px] text-ink-soft">
            아직 저장된 프로젝트가 없습니다. 위에서 업무를 시작해 보세요.
          </div>
        ) : (
          <div className="flex flex-col gap-2">
            {projects.map((p) => (
              <button
                key={p.id}
                type="button"
                onClick={() => void openProject(p)}
                title={p.workdir}
                className="flex items-center gap-3 rounded-[12px] border border-line bg-panel px-3.5 py-3 text-left shadow-sm transition-[transform,box-shadow] duration-[120ms] hover:-translate-y-0.5 hover:shadow-md"
              >
                <span className="grid h-9 w-9 shrink-0 place-items-center rounded-[10px] bg-accent-tint text-accent">
                  <FolderOpen size={17} />
                </span>
                <span className="min-w-0 flex-1">
                  <span className="block truncate text-[13.5px] font-medium text-ink-strong">
                    {p.title}
                  </span>
                  <span className="mt-0.5 block truncate text-[12px] text-ink-soft">
                    {p.sessionCount}개 세션 · {sessionTime(p.updatedAt)}
                  </span>
                </span>
              </button>
            ))}
          </div>
        )}
      </div>
    </div>
  );
}
