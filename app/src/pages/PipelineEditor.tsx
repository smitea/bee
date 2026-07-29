import { useMemo, useState } from "react";
import { Plus, AlertTriangle, Save, X } from "lucide-react";

import { useTabs } from "../state/tabsStore";
import { pipelineCreate } from "../ipc/pipelines";
import {
  PipelineGraph,
  type PipelineDefinition,
} from "../domain/pipeline";

const SAMPLE_DAG = JSON.stringify(
  {
    input: {
      datasource: "binance",
      method: "subscribe",
      args: { symbol: "BTC/USDT", interval: "5min" },
      output: "in",
    },
    handlers: [
      {
        id: "phase_a",
        name: "indicator_ema",
        params: { period: 14 },
        upstream: ["in"],
      },
    ],
    output: {
      adapter: "console_emit",
      method: "emit",
      args: {},
      upstream: "phase_a",
    },
    crossPipelineRefs: [],
  },
  null,
  2,
);

export function PipelineEditor() {
  const openTab = useTabs((s) => s.open);
  const close = useTabs((s) => s.close);

  const editorTabId = useTabs((s) => {
    const active = s.tabs.find((t) => t.id === s.activeId);
    return active && active.kind === "pipeline_editor" ? active.id : null;
  });

  const [name, setName] = useState("");
  const [dagJson, setDagJson] = useState(SAMPLE_DAG);
  const [error, setError] = useState<string | null>(null);
  const [busy, setBusy] = useState(false);

  const parsed: PipelineDefinition | null = useMemo(() => {
    try {
      const raw = JSON.parse(dagJson);
      if (!raw || typeof raw !== "object") return null;
      return {
        id: 0,
        name: name || "(preview)",
        input: raw.input ?? { datasource: "(none)", method: "subscribe", args: {}, output: "in" },
        handlers: Array.isArray(raw.handlers) ? raw.handlers : [],
        output: raw.output ?? { adapter: "(none)", method: "emit", args: {}, upstream: "in" },
        crossPipelineRefs: Array.isArray(raw.crossPipelineRefs) ? raw.crossPipelineRefs : [],
      };
    } catch {
      return null;
    }
  }, [dagJson, name]);

  const onSave = async () => {
    setError(null);
    const trimmed = name.trim();
    if (!trimmed) {
      setError("name is required");
      return;
    }
    try {
      JSON.parse(dagJson);
    } catch {
      setError("invalid JSON in dag_json");
      return;
    }
    setBusy(true);
    try {
      const created = await pipelineCreate(trimmed, dagJson);
      await openTab({
        kind: "pipeline",
        resourceId: String(created.id),
        title: created.name,
      });
      if (editorTabId !== null) {
        await close(editorTabId);
      }
    } catch (e) {
      setError(String(e));
    } finally {
      setBusy(false);
    }
  };

  const onCancel = async () => {
    if (editorTabId !== null) await close(editorTabId);
  };

  return (
    <div className="space-y-4">
      <header className="flex items-center justify-between">
        <div>
          <h1 className="text-xl font-semibold">New Pipeline</h1>
          <p className="text-xs text-gray-500 dark:text-neutral-400">
            Designer live preview · slide between forms to edit a DAG
          </p>
        </div>
        <div className="flex items-center gap-2">
          <button
            type="button"
            onClick={() => void onCancel()}
            className="flex items-center gap-1 px-3 py-1.5 text-xs rounded border border-gray-200 dark:border-neutral-700 hover:bg-gray-50 dark:hover:bg-neutral-700"
          >
            <X size={12} />
            Cancel
          </button>
          <button
            type="button"
            onClick={() => void onSave()}
            disabled={busy}
            className="flex items-center gap-1 px-3 py-1.5 text-xs rounded bg-accent-blue text-white hover:bg-accent-blue/90 disabled:opacity-50"
          >
            <Save size={12} />
            Save pipeline
          </button>
        </div>
      </header>

      <div className="grid grid-cols-1 lg:grid-cols-2 gap-4">
        <section className="bg-white dark:bg-neutral-800 rounded-lg border border-gray-200 dark:border-neutral-700 p-4 space-y-3">
          <div className="flex items-center gap-2 text-[10px] uppercase tracking-wider text-gray-500 dark:text-neutral-400">
            <Plus size={10} />
            <span>Form</span>
          </div>
          <label className="flex flex-col gap-1">
            <span className="text-[10px] text-gray-500 dark:text-neutral-400">Name</span>
            <input
              placeholder="pipeline name"
              value={name}
              onChange={(e) => setName(e.target.value)}
              className="px-2 py-1 text-xs rounded border border-gray-200 dark:border-neutral-700 bg-white dark:bg-neutral-900"
            />
          </label>
          <label className="flex flex-col gap-1">
            <span className="text-[10px] text-gray-500 dark:text-neutral-400">dag_json</span>
            <textarea
              aria-label="dag_json"
              value={dagJson}
              onChange={(e) => setDagJson(e.target.value)}
              rows={18}
              className="px-2 py-1 text-xs font-mono rounded border border-gray-200 dark:border-neutral-700 bg-white dark:bg-neutral-900 resize-y"
            />
          </label>
          {error && (
            <div
              role="alert"
              className="flex items-center gap-2 text-xs rounded-md bg-red-50 dark:bg-red-900/20 text-accent-red border border-red-200 dark:border-red-800 p-2"
            >
              <AlertTriangle size={12} />
              {error}
            </div>
          )}
        </section>

        <section className="bg-white dark:bg-neutral-800 rounded-lg border border-gray-200 dark:border-neutral-700 p-4">
          <div className="text-[10px] uppercase tracking-wider text-gray-500 dark:text-neutral-400 mb-3">
            Preview
          </div>
          {parsed ? (
            <PipelineGraph
              pipeline={parsed}
              onSelectInput={() => {}}
              onSelectOutput={() => {}}
              onSelectHandler={() => {}}
              onSelectCrossPipelineRef={() => {}}
            />
          ) : (
            <p className="text-xs text-gray-400">invalid JSON — cannot preview</p>
          )}
        </section>
      </div>
    </div>
  );
}
