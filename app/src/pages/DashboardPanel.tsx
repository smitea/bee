import { useEffect, useState } from "react";
import { AlertTriangle, Loader2, Pause, Play } from "lucide-react";

import { pipelineLatestResult } from "../ipc/pipelines";

const POLL_MS = 2000;

interface Props {
  addr: string;
  jobId: number;
  label: string;
}

type PanelState =
  | { kind: "loading" }
  | { kind: "data"; numeric: number; subtitle: string }
  | { kind: "error"; message: string }
  | { kind: "paused"; last: { numeric: number; subtitle: string } | null }
  | { kind: "empty" };

export function DashboardPanel({ addr, jobId, label }: Props) {
  const [state, setState] = useState<PanelState>({ kind: "loading" });
  const [paused, setPaused] = useState(false);

  useEffect(() => {
    if (paused) return;
    let cancelled = false;
    const fetchOnce = async () => {
      try {
        const result = await pipelineLatestResult(addr, jobId);
        if (cancelled) return;
        if (result === null) {
          setState({ kind: "empty" });
        } else {
          setState({
            kind: "data",
            numeric: result.numeric,
            subtitle: result.label,
          });
        }
      } catch (e) {
        if (cancelled) return;
        setState({ kind: "error", message: String(e) });
      }
    };
    void fetchOnce();
    const handle = setInterval(fetchOnce, POLL_MS);
    return () => {
      cancelled = true;
      clearInterval(handle);
    };
  }, [addr, jobId, paused]);

  const togglePause = () => {
    if (paused) {
      setState({ kind: "loading" });
    } else {
      const last = state.kind === "data" ? state : null;
      setState({ kind: "paused", last });
    }
    setPaused((p) => !p);
  };

  return (
    <article
      data-testid="dashboard-panel"
      className="bg-white dark:bg-neutral-800 rounded-lg border border-gray-200 dark:border-neutral-700 p-4 space-y-2"
    >
      <header className="flex items-center justify-between">
        <span className="text-[10px] uppercase tracking-wider text-gray-500 dark:text-neutral-400">
          {label}
        </span>
        <button
          type="button"
          onClick={togglePause}
          aria-label={paused ? "Resume polling" : "Pause polling"}
          className="p-1 rounded text-gray-400 hover:text-accent-blue"
        >
          {paused ? <Play size={11} /> : <Pause size={11} />}
        </button>
      </header>
      <Body state={state} />
    </article>
  );
}

function Body({ state }: { state: PanelState }) {
  switch (state.kind) {
    case "loading":
      return (
        <div className="flex items-center gap-2 text-sm text-gray-500">
          <Loader2 size={14} className="animate-spin" />
          <span>loading…</span>
        </div>
      );
    case "data":
      return (
        <div>
          <div data-testid="dashboard-numeric" className="text-3xl font-semibold tabular-nums">
            {state.numeric.toFixed(2)}
          </div>
          <div className="text-[10px] text-gray-500 dark:text-neutral-400">{state.subtitle}</div>
        </div>
      );
    case "error":
      return (
        <div
          role="alert"
          className="flex items-center gap-2 text-xs rounded-md bg-red-50 dark:bg-red-900/20 text-accent-red border border-red-200 dark:border-red-800 p-2"
        >
          <AlertTriangle size={12} />
          <span>failed: {state.message}</span>
        </div>
      );
    case "paused":
      return (
        <div>
          {state.last && (
            <div className="text-3xl font-semibold tabular-nums text-gray-400">
              {state.last.numeric.toFixed(2)}
            </div>
          )}
          <div data-testid="paused-state" className="text-xs text-gray-400 italic">
            paused
          </div>
        </div>
      );
    case "empty":
      return <div className="text-xs text-gray-400">no data</div>;
  }
}
