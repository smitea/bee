import { useState } from "react";
import { useQuery } from "@tanstack/react-query";
import { RefreshCcw, Workflow, AlertTriangle } from "lucide-react";
import { useStore } from "../state/store";
import {
  jobInspect,
  listJobs,
  type JobDetail,
  type JobSummary,
} from "../ipc";

const REFRESH_MS = 5000;

export function Pipelines() {
  const addr = useStore((s) => s.addr);
  const jobsQ = useQuery<JobSummary[]>({
    queryKey: ["jobs", addr],
    queryFn: () => listJobs(addr),
    refetchInterval: REFRESH_MS,
  });
  const [selected, setSelected] = useState<number | null>(null);
  const detailQ = useQuery<JobDetail | null>({
    queryKey: ["job", addr, selected],
    queryFn: () => (selected ? jobInspect(addr, selected) : Promise.resolve(null)),
    enabled: selected !== null,
  });

  const lifeColor = (s: string) =>
    s === "Running"
      ? "bg-accent-green"
      : s === "Completed"
        ? "bg-accent-blue"
        : s === "Failed"
          ? "bg-accent-red"
          : s === "WaitingForUpstream"
            ? "bg-accent-orange"
            : "bg-gray-400";

  return (
    <div className="space-y-6">
      <div className="flex items-center justify-between">
        <h1 className="text-xl font-semibold">Pipelines</h1>
        <button
          onClick={() => jobsQ.refetch()}
          disabled={jobsQ.isFetching}
          className="flex items-center gap-1.5 px-3 py-1.5 text-xs rounded-md bg-white dark:bg-neutral-800 border border-gray-200 dark:border-neutral-700 hover:bg-gray-50 dark:hover:bg-neutral-700 disabled:opacity-50"
        >
          <RefreshCcw size={14} className={jobsQ.isFetching ? "animate-spin" : ""} />
          Refresh
        </button>
      </div>

      {jobsQ.error && (
        <div className="flex items-center gap-2 px-3 py-2 text-sm rounded-md bg-red-50 dark:bg-red-900/20 text-accent-red border border-red-200 dark:border-red-800">
          <AlertTriangle size={14} />
          RPC error: {String(jobsQ.error)}
        </div>
      )}

      <div className="bg-white dark:bg-neutral-800 rounded-lg border border-gray-200 dark:border-neutral-700">
        <h2 className="px-4 py-3 text-sm font-medium border-b border-gray-200 dark:border-neutral-700">
          Jobs
        </h2>
        <div className="p-4">
          {jobsQ.data?.length === 0 ? (
            <div className="text-center py-8 text-gray-400">
              <Workflow
                size={48}
                className="mx-auto text-gray-300 dark:text-neutral-600"
              />
              <p className="mt-2 text-sm">no jobs — click Refresh</p>
            </div>
          ) : (
            <table className="w-full text-xs">
              <thead>
                <tr className="text-[10px] uppercase tracking-wider text-gray-500 dark:text-neutral-400">
                  <th className="text-left font-medium pb-2 pr-4">Job</th>
                  <th className="text-left font-medium pb-2 pr-4">Lifecycle</th>
                  <th className="text-left font-medium pb-2 pr-4">Mode</th>
                  <th className="text-left font-medium pb-2 pr-4">Tasks</th>
                  <th className="text-left font-medium pb-2 pr-4">Node</th>
                  <th className="text-left font-medium pb-2 pr-4">Actions</th>
                </tr>
              </thead>
              <tbody>
                {jobsQ.data?.map((j) => (
                  <tr
                    key={j.job_id}
                    className="border-t border-gray-100 dark:border-neutral-800"
                  >
                    <td className="py-2 pr-4 font-mono">
                      <span
                        className={`inline-block w-2 h-2 rounded-full mr-2 align-middle ${lifeColor(j.lifecycle)}`}
                      />
                      #{j.job_id}
                    </td>
                    <td className="py-2 pr-4 text-gray-600 dark:text-neutral-300">
                      {j.lifecycle}
                    </td>
                    <td className="py-2 pr-4">{j.mode}</td>
                    <td className="py-2 pr-4 font-mono">{j.task_count}</td>
                    <td className="py-2 pr-4 font-mono">#{j.owner_node}</td>
                    <td className="py-2">
                      <button
                        onClick={() => setSelected(j.job_id)}
                        className="px-2 py-0.5 text-[11px] rounded border border-gray-200 dark:border-neutral-700 hover:bg-gray-50 dark:hover:bg-neutral-700"
                      >
                        Inspect
                      </button>
                    </td>
                  </tr>
                ))}
              </tbody>
            </table>
          )}
        </div>
      </div>

      {selected !== null && (
        <div className="bg-white dark:bg-neutral-800 rounded-lg border border-gray-200 dark:border-neutral-700">
          <h2 className="px-4 py-3 text-sm font-medium border-b border-gray-200 dark:border-neutral-700 flex items-center justify-between">
            <span>Inspect: #{selected}</span>
            <button
              onClick={() => setSelected(null)}
              className="px-2 py-1 text-xs rounded-md border border-gray-200 dark:border-neutral-700 hover:bg-gray-50 dark:hover:bg-neutral-700"
            >
              Close
            </button>
          </h2>
          <div className="p-4 text-xs space-y-1 max-h-72 overflow-y-auto">
            {detailQ.isLoading && <p className="text-gray-500">loading…</p>}
            {detailQ.data && (
              <>
                <Row label="dag_hash">{detailQ.data.dag_hash}</Row>
                <Row label="lifecycle">{detailQ.data.lifecycle}</Row>
                <Row label="owner_node">#{detailQ.data.owner_node}</Row>
                <Row label="deps">
                  {detailQ.data.dependencies.length} cross-pipeline edge(s)
                </Row>
                {detailQ.data.dependencies.map((d, i) => (
                  <p key={i} className="pl-32 text-gray-500">
                    ↑ job {d.upstream_job} stream {d.stream}
                  </p>
                ))}
                <Row label="tasks">
                  {detailQ.data.tasks.length} task(s)
                </Row>
                {detailQ.data.tasks.map((t) => (
                  <p key={t.task_id} className="pl-32 text-gray-500 font-mono text-[11px]">
                    task #{t.task_id} phase {t.phase_id} owner #{t.owner_node} status {t.status}
                  </p>
                ))}
              </>
            )}
          </div>
        </div>
      )}
    </div>
  );
}

function Row({ label, children }: { label: string; children: React.ReactNode }) {
  return (
    <div className="flex items-start gap-3 py-0.5">
      <span className="w-32 text-gray-600 dark:text-neutral-400">{label}:</span>
      <span className="font-mono text-[11px]">{children}</span>
    </div>
  );
}