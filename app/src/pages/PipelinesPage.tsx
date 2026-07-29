import { useMemo, useState } from "react";
import { useQuery } from "@tanstack/react-query";
import { Plus, Workflow, AlertTriangle, Trash2 } from "lucide-react";

import { useConnection } from "../state/connectionStore";
import { useTabs } from "../state/tabsStore";
import {
  pipelineList,
  pipelineCreate,
  pipelineDelete,
  listJobs,
  type PipelineDefinitionView,
  type JobSummary,
} from "../ipc";

const REFRESH_MS = 5000;

type SectionKey = "queued" | "running" | "historical" | "failed";

function sectionOf(lifecycle: string): SectionKey {
  switch (lifecycle) {
    case "WaitingForUpstream":
      return "queued";
    case "Running":
    case "Restarting":
      return "running";
    case "Completed":
      return "historical";
    case "Failed":
      return "failed";
    default:
      return "historical";
  }
}

const SECTION_LABEL: Record<SectionKey, string> = {
  queued: "Queued",
  running: "Running",
  historical: "Historical",
  failed: "Failed",
};

const SECTION_ORDER: SectionKey[] = ["queued", "running", "historical", "failed"];

export function PipelinesPage() {
  const openTab = useTabs((s) => s.open);
  const addr = useConnection((s) => s.addr);

  const jobsQ = useQuery<JobSummary[]>({
    queryKey: ["jobs", addr],
    queryFn: () => listJobs(addr),
    refetchInterval: REFRESH_MS,
  });

  const defsQ = useQuery<PipelineDefinitionView[]>({
    queryKey: ["pipeline-defs"],
    queryFn: () => pipelineList(),
  });

  const [draft, setDraft] = useState({ name: "", dag_json: "{}" });
  const [error, setError] = useState<string | null>(null);
  const [busy, setBusy] = useState(false);

  const sections = useMemo(() => {
    const out: Record<SectionKey, JobSummary[]> = {
      queued: [],
      running: [],
      historical: [],
      failed: [],
    };
    for (const j of jobsQ.data ?? []) {
      out[sectionOf(j.lifecycle)].push(j);
    }
    return out;
  }, [jobsQ.data]);

  const onCreate = async () => {
    setError(null);
    const name = draft.name.trim();
    if (!name) {
      setError("name is required");
      return;
    }
    try {
      JSON.parse(draft.dag_json);
    } catch {
      setError("dag_json must be valid JSON");
      return;
    }
    setBusy(true);
    try {
      const created = await pipelineCreate(name, draft.dag_json);
      setDraft({ name: "", dag_json: "{}" });
      await defsQ.refetch();
      await openTab({
        kind: "pipeline",
        resourceId: String(created.id),
        title: created.name,
      });
    } catch (e) {
      setError(String(e));
    } finally {
      setBusy(false);
    }
  };

  const onDelete = async (id: number) => {
    if (!confirm(`Delete pipeline #${id}?`)) return;
    await pipelineDelete(id);
    await defsQ.refetch();
  };

  return (
    <div className="space-y-6">
      <header className="flex items-center justify-between">
        <h1 className="text-xl font-semibold">Pipelines</h1>
        <div className="flex items-center gap-2">
          <button
            type="button"
            onClick={() => void openTab({ kind: "pipeline_editor", resourceId: null, title: "New Pipeline" })}
            className="flex items-center gap-1.5 px-3 py-1.5 text-xs rounded-md border border-gray-200 dark:border-neutral-700 hover:bg-gray-50 dark:hover:bg-neutral-700"
          >
            <Plus size={14} />
            New Pipeline
          </button>
          <button
            type="button"
            onClick={() => void onCreate()}
            disabled={busy}
            className="flex items-center gap-1.5 px-3 py-1.5 text-xs rounded-md bg-accent-blue text-white hover:bg-accent-blue/90 disabled:opacity-50"
          >
            <Plus size={14} />
            Create pipeline
          </button>
        </div>
      </header>

      {(jobsQ.error || defsQ.error) && (
        <div className="flex items-center gap-2 px-3 py-2 text-sm rounded-md bg-red-50 dark:bg-red-900/20 text-accent-red border border-red-200 dark:border-red-800">
          <AlertTriangle size={14} />
          RPC error: {String(jobsQ.error || defsQ.error)}
        </div>
      )}

      <div className="bg-white dark:bg-neutral-800 rounded-lg border border-gray-200 dark:border-neutral-700">
        <h2 className="px-4 py-3 text-sm font-medium border-b border-gray-200 dark:border-neutral-700">
          New Pipeline Definition
        </h2>
        <div className="p-4 grid grid-cols-1 md:grid-cols-3 gap-3 text-xs">
          <label className="flex flex-col gap-1">
            <span className="text-gray-500 dark:text-neutral-400">Name</span>
            <input
              value={draft.name}
              onChange={(e) => setDraft({ ...draft, name: e.target.value })}
              placeholder="btc_kline"
              className="px-2 py-1 rounded border border-gray-200 dark:border-neutral-700 bg-white dark:bg-neutral-900"
            />
          </label>
          <label className="flex flex-col gap-1 md:col-span-2">
            <span className="text-gray-500 dark:text-neutral-400">dag_json</span>
            <input
              value={draft.dag_json}
              onChange={(e) => setDraft({ ...draft, dag_json: e.target.value })}
              className="px-2 py-1 rounded border border-gray-200 dark:border-neutral-700 bg-white dark:bg-neutral-900 font-mono"
            />
          </label>
          {error && (
            <div className="md:col-span-3 text-accent-red">{error}</div>
          )}
        </div>
      </div>

      <div className="bg-white dark:bg-neutral-800 rounded-lg border border-gray-200 dark:border-neutral-700">
        <h2 className="px-4 py-3 text-sm font-medium border-b border-gray-200 dark:border-neutral-700">
          Definitions ({defsQ.data?.length ?? 0})
        </h2>
        <div className="p-4">
          {(defsQ.data?.length ?? 0) === 0 ? (
            <p className="text-xs text-gray-400 text-center py-4">no saved pipelines</p>
          ) : (
            <ul className="text-xs space-y-1">
              {defsQ.data?.map((d) => (
                <li
                  key={d.id}
                  className="flex items-center justify-between px-2 py-1 rounded hover:bg-gray-50 dark:hover:bg-neutral-700/50"
                >
                  <button
                    type="button"
                    onClick={() =>
                      void openTab({
                        kind: "pipeline",
                        resourceId: String(d.id),
                        title: d.name,
                      })
                    }
                    className="flex-1 text-left font-mono"
                  >
                    {d.name}
                  </button>
                  <button
                    type="button"
                    onClick={() => void onDelete(d.id)}
                    aria-label={`delete pipeline ${d.name}`}
                    className="p-1 rounded text-gray-400 hover:text-accent-red"
                  >
                    <Trash2 size={12} />
                  </button>
                </li>
              ))}
            </ul>
          )}
        </div>
      </div>

      <div className="grid grid-cols-1 md:grid-cols-2 xl:grid-cols-4 gap-3">
        {SECTION_ORDER.map((k) => (
          <section
            key={k}
            className="bg-white dark:bg-neutral-800 rounded-lg border border-gray-200 dark:border-neutral-700"
          >
            <h3 className="px-3 py-2 text-xs font-semibold border-b border-gray-200 dark:border-neutral-700">
              {SECTION_LABEL[k]} ({sections[k].length})
            </h3>
            <div className="p-2 min-h-[5rem]">
              {sections[k].length === 0 ? (
                <p className="text-[10px] text-gray-400 text-center py-4">
                  <Workflow size={20} className="mx-auto text-gray-300 dark:text-neutral-600" />
                  empty
                </p>
              ) : (
                <ul className="space-y-1 text-xs">
                  {sections[k].map((j) => (
                    <li
                      key={j.job_id}
                      className="px-2 py-1 rounded hover:bg-gray-50 dark:hover:bg-neutral-700/50 flex items-center gap-2"
                    >
                      <span className="font-mono">#{j.job_id}</span>
                      <span className="text-gray-500 dark:text-neutral-400 text-[10px] truncate">
                        {j.dag_hash.slice(0, 8)}
                      </span>
                    </li>
                  ))}
                </ul>
              )}
            </div>
          </section>
        ))}
      </div>
    </div>
  );
}
