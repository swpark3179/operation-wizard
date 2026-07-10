import { Component, type ErrorInfo, type ReactNode } from "react";
import { AlertTriangle, RefreshCw, RotateCcw } from "lucide-react";

interface Props {
  children: ReactNode;
}

interface State {
  error: Error | null;
}

/**
 * Render-error containment (D56). Without a boundary, one thrown render
 * unmounts the entire React root — a permanent white screen with no recovery.
 * Class component: boundaries have no hooks API. Note this only catches
 * render/lifecycle errors; event handlers and async paths use try/catch.
 */
export class ErrorBoundary extends Component<Props, State> {
  state: State = { error: null };

  static getDerivedStateFromError(error: Error): State {
    return { error };
  }

  componentDidCatch(error: Error, info: ErrorInfo) {
    console.error("[operation-wizard] render error", error, info);
  }

  private reset = () => this.setState({ error: null });

  render() {
    const { error } = this.state;
    if (!error) return this.props.children;

    return (
      <div className="grid h-full w-full place-items-center bg-app p-6 text-ink">
        <div className="flex w-full max-w-md flex-col items-center gap-3 text-center">
          <AlertTriangle size={28} className="text-bad" />
          <div className="text-[15px] font-semibold text-ink-strong">
            문제가 발생했습니다
          </div>
          <p className="text-[13px] leading-[1.6] text-ink-muted">
            화면을 그리는 중 오류가 발생했습니다. 다시 시도해도 반복되면 앱을
            새로고침해 주세요.
          </p>
          <div className="max-h-32 w-full overflow-auto rounded-lg border border-line bg-subtle px-3 py-2 text-left font-mono text-[12px] leading-[1.5] text-ink-muted">
            {error.message || String(error)}
          </div>
          <div className="mt-1 flex items-center gap-2">
            <button
              type="button"
              onClick={this.reset}
              className="flex items-center gap-1.5 rounded-[6px] bg-accent px-3.5 py-1.5 text-[13px] font-medium text-white transition-colors duration-[120ms] hover:bg-accent-strong"
            >
              <RotateCcw size={14} />
              다시 시도
            </button>
            <button
              type="button"
              onClick={() => window.location.reload()}
              className="flex items-center gap-1.5 rounded-[6px] border border-line bg-panel px-3.5 py-1.5 text-[13px] font-medium text-ink-muted transition-colors duration-[120ms] hover:bg-subtle hover:text-ink"
            >
              <RefreshCw size={14} />
              앱 새로고침
            </button>
          </div>
        </div>
      </div>
    );
  }
}
