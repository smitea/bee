import { useMemo, useState } from "react";
import { useQuery } from "@tanstack/react-query";
import {
  Server,
  Network,
  AlertTriangle,
  RefreshCw,
  RotateCw,
  X,
} from "lucide-react";

import { useConnection } from "../state/connectionStore";
import { useUi } from "../state/store";
import {
  clusterStatus,
  listJobs,
  rollingRestartApply,
  type ClusterMetrics,
  type JobSummary,
  type RollingRestartPlan,
} from "../ipc";
import { ClusterTopology, type TopologyNode } from "../components/ClusterTopology";

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

export function ClusterDashboard() {
  const addr = useConnection((s) => s.addr);
  const openSettings = useUi((s) => s.openSettings);
  const clusterQ = useQuery<ClusterMetrics>({
    queryKey: ["cluster", addr],
    queryFn: () => clusterStatus(addr),
    refetchInterval: REFRESH_MS,
  });
  const jobsQ = useQuery<JobSummary[]>({
    queryKey: ["jobs", addr],
    queryFn: () => listJobs(addr),
    refetchInterval: REFRESH_MS,
  });

  const counts = useMemo(() => {
    const out: Record<SectionKey, number> = {
      queued: 0,
      running: 0,
      historical: 0,
      failed: 0,
    };
    for (const j of jobsQ.data ?? []) out[sectionOf(j.lifecycle)] += 1;
    return out;
  }, [jobsQ.data]);

  const longestRunning = useMemo(() => {
    const running = (jobsQ.data ?? []).filter(
      (j) => j.lifecycle === "Running" || j.lifecycle === "Restarting",
    );
    if (running.length === 0) return null;
    return [...running].sort((a, b) => a.job_id - b.job_id)[0];
  }, [jobsQ.data]);

  const highestResource = useMemo(() => {
    if ((jobsQ.data ?? []).length === 0) return null;
    return [...(jobsQ.data ?? [])].sort(
      (a, b) => b.task_count - a.task_count || a.job_id - b.job_id,
    )[0];
  }, [jobsQ.data]);

  const averageRuntimeLabel = "no data";

  const [restartState, setRestartState] = useState<"idle" | "running" | "done" | "error">("idle");
  const [planDialog, setPlanDialog] = useState<RollingRestartPlan | null>(null);

  const onRollingRestart = async () => {
    setRestartState("running");
    try {
      const nodeAddrs = (clusterQ.data?.nodes ?? []).map((n) => ({
        id: `n${n.id}`,
        addr: addr,
      }));
      const plan = await rollingRestartApply(addr, nodeAddrs);
      setPlanDialog(plan);
      setRestartState("done");
    } catch {
      setRestartState("error");
    }
  };

  const quorumOk = (clusterQ.data?.nodes.length ?? 0) >= 2;
  const leader = clusterQ.data?.leader_id ?? null;
  const term = clusterQ.data?.term ?? 0;

  return (
    <div className="space-y-6">
      {(clusterQ.error || jobsQ.error) && (
        <div className="flex items-center gap-2 px-3 py-2 text-sm rounded-md bg-red-50 dark:bg-red-900/20 text-accent-red border border-red-200 dark:border-red-800">
          <AlertTriangle size={14} />
          RPC error: {String(clusterQ.error || jobsQ.error)}
        </div>
      )}

      <section className="bg-white dark:bg-neutral-800 rounded-lg border border-gray-200 dark:border-neutral-700">
        <h2 className="px-4 py-3 text-sm font-semibold border-b border-gray-200 dark:border-neutral-700">
          Topology
        </h2>
        <div className="p-4 grid grid-cols-1 md:grid-cols-3 gap-3">
          <div className="rounded-md border border-gray-200 dark:border-neutral-700 p-3">
            <div className="text-[10px] uppercase tracking-wider text-gray-500 dark:text-neutral-400">
              Raft Leader
            </div>
            <div className="text-xl font-semibold">
              {leader === null ? "no data" : `#${leader}`}
            </div>
            <div className="text-[10px] text-gray-500 dark:text-neutral-400">
              term {term}
            </div>
          </div>
          <div className="rounded-md border border-gray-200 dark:border-neutral-700 p-3">
            <div className="text-[10px] uppercase tracking-wider text-gray-500 dark:text-neutral-400">
              Quorum health
            </div>
            <div
              className={[
                "text-xl font-semibold",
                quorumOk ? "text-accent-green" : "text-accent-red",
              ].join(" ")}
            >
              {clusterQ.data ? (quorumOk ? "healthy" : "no quorum") : "no data"}
            </div>
            <div className="text-[10px] text-gray-500 dark:text-neutral-400">
              {clusterQ.data?.nodes.length ?? 0} nodes
            </div>
          </div>
          <div className="rounded-md border border-gray-200 dark:border-neutral-700 p-3">
            <div className="text-[10px] uppercase tracking-wider text-gray-500 dark:text-neutral-400">
              Commit index
            </div>
            <div className="text-xl font-mono">
              {clusterQ.data?.commit_index ?? "no data"}
            </div>
            <div className="text-[10px] text-gray-500 dark:text-neutral-400">
              log length {clusterQ.data?.nodes[0]?.log_length ?? "—"}
            </div>
          </div>
        </div>
        <div className="px-4 pb-4">
          <ClusterTopology
            nodes={(clusterQ.data?.nodes ?? []).map<TopologyNode>((n) => ({
              id: n.id,
              role: n.role as TopologyNode["role"],
              addr: addr,
              term: clusterQ.data?.term ?? 0,
              lag: 0,
              error: n.role === "Follower" && n.log_length < (clusterQ.data?.commit_index ?? 0)
                ? `lag ${(clusterQ.data?.commit_index ?? 0) - n.log_length}`
                : null,
            }))}
            leaderId={clusterQ.data?.leader_id ?? null}
            onSelectCluster={openSettings}
          />
        </div>
        <div className="px-4 pb-4">
          <table className="w-full text-xs">
            <thead>
              <tr className="text-[10px] uppercase tracking-wider text-gray-500 dark:text-neutral-400">
                <th className="text-left font-medium pb-2">Node</th>
                <th className="text-left font-medium pb-2">Role</th>
                <th className="text-left font-medium pb-2">Commit</th>
                <th className="text-left font-medium pb-2">Log length</th>
              </tr>
            </thead>
            <tbody>
              {(clusterQ.data?.nodes ?? []).map((n) => (
                <tr
                  key={n.id}
                  className="border-t border-gray-100 dark:border-neutral-800"
                  data-testid={`node-row-${n.id}`}
                >
                  <td className="py-2 font-mono">node #{n.id}</td>
                  <td className="py-2">
                    <span
                      className={[
                        "px-1.5 py-0.5 rounded text-[10px] font-semibold uppercase",
                        n.role === "Leader"
                          ? "bg-accent-blue text-white"
                          : n.role === "Candidate"
                            ? "bg-accent-orange text-white"
                            : "bg-gray-500 text-white",
                      ].join(" ")}
                    >
                      {n.role}
                    </span>
                  </td>
                  <td className="py-2 font-mono">{n.commit_index}</td>
                  <td className="py-2 font-mono">{n.log_length}</td>
                </tr>
              ))}
              {(clusterQ.data?.nodes.length ?? 0) === 0 && (
                <tr>
                  <td className="py-2 text-gray-400" colSpan={4}>
                    {clusterQ.isLoading ? "loading…" : "no data"}
                  </td>
                </tr>
              )}
            </tbody>
          </table>
        </div>
      </section>

      <section>
        <h2 className="text-sm font-semibold text-gray-700 dark:text-neutral-200 mb-2">
          Pipeline Jobs
        </h2>
        <div className="grid grid-cols-2 md:grid-cols-4 gap-3">
          {(Object.keys(SECTION_LABEL) as SectionKey[]).map((k) => (
            <article
              key={k}
              data-testid={`pipeline-jobs-${k}`}
              className="bg-white dark:bg-neutral-800 rounded-lg border border-gray-200 dark:border-neutral-700 p-3"
            >
              <div className="flex items-center gap-2">
                <Server size={12} className="text-gray-400" />
                <span className="text-[10px] uppercase tracking-wider text-gray-500 dark:text-neutral-400">
                  {SECTION_LABEL[k]}
                </span>
              </div>
              <div className="mt-1 text-2xl font-semibold tabular-nums">
                {counts[k]}
              </div>
            </article>
          ))}
        </div>
      </section>

      <section className="grid grid-cols-1 md:grid-cols-3 gap-3">
        <MetricCard
          icon={<Network size={12} />}
          label="Longest-running Pipeline Job"
          testId="metric-longest"
          value={
            longestRunning
              ? `#${longestRunning.job_id}`
              : "no data"
          }
          sub={longestRunning ? longestRunning.dag_hash.slice(0, 10) : "—"}
        />
        <MetricCard
          icon={<Network size={12} />}
          label="Highest-resource Pipeline Job"
          testId="metric-resource"
          value={
            highestResource
              ? `#${highestResource.job_id}`
              : "no data"
          }
          sub={
            highestResource
              ? `${highestResource.task_count} tasks`
              : "—"
          }
        />
        <MetricCard
          icon={<Network size={12} />}
          label="Average runtime"
          testId="metric-runtime"
          value={averageRuntimeLabel}
          sub="aggregate · slice 1.x"
        />
      </section>

      <section className="bg-white dark:bg-neutral-800 rounded-lg border border-gray-200 dark:border-neutral-700">
        <h2 className="px-4 py-3 text-sm font-semibold border-b border-gray-200 dark:border-neutral-700">
          Active configuration
        </h2>
        <div className="p-4 grid grid-cols-1 md:grid-cols-2 gap-3 text-xs">
          <div className="rounded border border-gray-200 dark:border-neutral-700 p-3">
            <div className="text-gray-500 dark:text-neutral-400">Address</div>
            <div className="font-mono">{addr}</div>
          </div>
          <div className="rounded border border-gray-200 dark:border-neutral-700 p-3 flex items-center gap-3">
            <div className="flex-1">
              <div className="text-gray-500 dark:text-neutral-400">
                Rolling restart ops
              </div>
              <div className="text-[10px] text-gray-400">
                round-trip a ping to each node (slice 1.x fans out)
              </div>
            </div>
            <button
              type="button"
              data-testid="rolling-restart"
              onClick={() => void onRollingRestart()}
              disabled={restartState === "running"}
              className="flex items-center gap-1.5 px-3 py-1.5 rounded bg-accent-blue text-white hover:bg-accent-blue/90 disabled:opacity-50"
            >
              {restartState === "running" ? (
                <RefreshCw size={12} className="animate-spin" />
              ) : (
                <RotateCw size={12} />
              )}
              Rolling restart
            </button>
          </div>
          {restartState !== "idle" && (
            <div
              className={[
                "md:col-span-2 text-[11px]",
                restartState === "done"
                  ? "text-accent-green"
                  : restartState === "error"
                    ? "text-accent-red"
                    : "text-gray-500",
              ].join(" ")}
            >
              {restartState === "running"
                ? "planning…"
                : restartState === "done"
                  ? "plan ready"
                  : "plan failed"}
            </div>
          )}
        </div>
      </section>

      {planDialog && (
        <div
          className="fixed inset-0 z-50 flex items-center justify-center bg-black/40"
          role="dialog"
          aria-modal="true"
          aria-label="Rolling restart plan"
          data-testid="rolling-restart-plan"
        >
          <div className="bg-white dark:bg-neutral-800 rounded-lg shadow-xl w-[520px] max-w-[95vw] max-h-[90vh] flex flex-col">
            <header className="flex items-center justify-between px-4 py-3 border-b border-gray-200 dark:border-neutral-700">
              <h2 className="text-sm font-semibold">Rolling restart plan</h2>
              <button
                type="button"
                onClick={() => setPlanDialog(null)}
                aria-label="Close"
                className="p-1 rounded text-gray-500 hover:bg-gray-100 dark:hover:bg-neutral-700"
              >
                <X size={14} />
              </button>
            </header>
            <div className="p-4 space-y-3 text-xs">
              <div className="text-gray-500 dark:text-neutral-400">
                batch={planDialog.batch_size} · timeout_ms={planDialog.health_timeout_ms}
              </div>
              <table className="w-full text-xs">
                <thead>
                  <tr className="text-[10px] uppercase tracking-wider text-gray-500 dark:text-neutral-400">
                    <th className="text-left font-medium pb-2">Order</th>
                    <th className="text-left font-medium pb-2">Node</th>
                    <th className="text-left font-medium pb-2">Address</th>
                  </tr>
                </thead>
                <tbody>
                  {planDialog.nodes.map((n, idx) => (
                    <tr key={n.id} className="border-t border-gray-100 dark:border-neutral-800">
                      <td className="py-2 font-mono">{idx + 1}</td>
                      <td className="py-2 font-mono">{n.id}</td>
                      <td className="py-2 font-mono">{n.addr}</td>
                    </tr>
                  ))}
                </tbody>
              </table>
            </div>
            <footer className="px-4 py-3 border-t border-gray-200 dark:border-neutral-700 flex items-center justify-end gap-2">
              <button
                type="button"
                onClick={() => setPlanDialog(null)}
                className="px-3 py-1 text-xs rounded border border-gray-200 dark:border-neutral-700 hover:bg-gray-50 dark:hover:bg-neutral-700"
              >
                Close
              </button>
            </footer>
          </div>
        </div>
      )}
    </div>
  );
}

function MetricCard({
  icon,
  label,
  value,
  sub,
  testId,
}: {
  icon: React.ReactNode;
  label: string;
  value: string;
  sub: string;
  testId?: string;
}) {
  return (
    <article
      data-testid={testId}
      className="bg-white dark:bg-neutral-800 rounded-lg border border-gray-200 dark:border-neutral-700 p-3"
    >
      <div className="flex items-center gap-2 text-gray-500 dark:text-neutral-400 text-[10px] uppercase tracking-wider">
        {icon}
        <span>{label}</span>
      </div>
      <div className="mt-1 text-xl font-semibold tabular-nums">{value}</div>
      <div className="text-[10px] text-gray-400 font-mono">{sub}</div>
    </article>
  );
}