import { useQuery } from "@tanstack/react-query";
import { RefreshCcw, Network, Workflow, Activity, AlertTriangle } from "lucide-react";
import { useStore } from "../state/store";
import {
  clusterStatus,
  listJobs,
  type ClusterMetrics,
  type JobSummary,
} from "../ipc";

const REFRESH_MS = 5000;

export function Dashboard() {
  const addr = useStore((s) => s.addr);
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

  const refreshing = clusterQ.isFetching || jobsQ.isFetching;

  return (
    <div className="space-y-6">
      <div className="flex items-center justify-between">
        <h1 className="text-xl font-semibold">Dashboard</h1>
        <button
          onClick={() => {
            clusterQ.refetch();
            jobsQ.refetch();
          }}
          disabled={refreshing}
          className="flex items-center gap-1.5 px-3 py-1.5 text-xs rounded-md bg-white dark:bg-neutral-800 border border-gray-200 dark:border-neutral-700 text-gray-700 dark:text-neutral-200 hover:bg-gray-50 dark:hover:bg-neutral-700 disabled:opacity-50"
        >
          <RefreshCcw size={14} className={refreshing ? "animate-spin" : ""} />
          Refresh
        </button>
      </div>

      {(clusterQ.error || jobsQ.error) && (
        <div className="flex items-center gap-2 px-3 py-2 text-sm rounded-md bg-red-50 dark:bg-red-900/20 text-accent-red border border-red-200 dark:border-red-800">
          <AlertTriangle size={14} />
          RPC error: {String(clusterQ.error || jobsQ.error)}
        </div>
      )}

      <div className="grid grid-cols-3 gap-4">
        <StatCard
          Icon={Network}
          title="Cluster"
          big={clusterQ.data ? `${clusterQ.data.nodes.length}` : "—"}
          sub={
            clusterQ.data
              ? `leader ${clusterQ.data.leader_id ?? "—"} · term ${clusterQ.data.term} · commit ${clusterQ.data.commit_index}`
              : clusterQ.isLoading
                ? "loading…"
                : "click Refresh"
          }
        />
        <StatCard
          Icon={Workflow}
          title="Jobs"
          big={`${jobsQ.data?.length ?? 0}`}
          sub={`${jobsQ.data?.filter((j) => j.lifecycle === "Running").length ?? 0} running`}
        />
        <StatCard
          Icon={Activity}
          title="Tasks"
          big={`${jobsQ.data?.reduce((sum, j) => sum + j.task_count, 0) ?? 0}`}
          sub={`${jobsQ.data?.filter((j) => j.lifecycle === "Completed").length ?? 0} completed`}
        />
      </div>

      <Section title="Nodes">
        <Table
          headers={["ID", "Role", "Commit", "Log length"]}
          empty={clusterQ.data ? null : "click Refresh"}
        >
          {clusterQ.data?.nodes.map((n) => (
            <tr key={n.id} className="border-t border-gray-100 dark:border-neutral-800">
              <td className="py-2 pr-4">
                <span
                  className={[
                    "inline-block w-2 h-2 rounded-full mr-2 align-middle",
                    n.role === "Leader"
                      ? "bg-accent-blue"
                      : n.role === "Candidate"
                        ? "bg-accent-orange"
                        : "bg-gray-400",
                  ].join(" ")}
                />
                <span className="font-mono">node #{n.id}</span>
              </td>
              <td className="py-2 pr-4 text-gray-600 dark:text-neutral-300">
                {n.role}
              </td>
              <td className="py-2 pr-4 font-mono">{n.commit_index}</td>
              <td className="py-2 font-mono">{n.log_length}</td>
            </tr>
          ))}
        </Table>
      </Section>

      <Section title="Recent Jobs">
        <Table
          headers={["Job", "Lifecycle", "Mode", "Tasks", "Node"]}
          empty={jobsQ.data?.length === 0 ? "no jobs" : null}
        >
          {jobsQ.data?.map((j) => (
            <tr key={j.job_id} className="border-t border-gray-100 dark:border-neutral-800">
              <td className="py-2 pr-4">
                <span
                  className={[
                    "inline-block w-2 h-2 rounded-full mr-2 align-middle",
                    j.lifecycle === "Running"
                      ? "bg-accent-green"
                      : j.lifecycle === "Completed"
                        ? "bg-accent-blue"
                        : j.lifecycle === "Failed"
                          ? "bg-accent-red"
                          : j.lifecycle === "WaitingForUpstream"
                            ? "bg-accent-orange"
                            : "bg-gray-400",
                  ].join(" ")}
                />
                <span className="font-mono">#{j.job_id}</span>
              </td>
              <td className="py-2 pr-4 text-gray-600 dark:text-neutral-300">
                {j.lifecycle}
              </td>
              <td className="py-2 pr-4">{j.mode}</td>
              <td className="py-2 pr-4 font-mono">{j.task_count}</td>
              <td className="py-2 font-mono">#{j.owner_node}</td>
            </tr>
          ))}
        </Table>
      </Section>
    </div>
  );
}

function StatCard({
  Icon,
  title,
  big,
  sub,
}: {
  Icon: typeof Network;
  title: string;
  big: string;
  sub: string;
}) {
  return (
    <div className="bg-white dark:bg-neutral-800 rounded-lg border border-gray-200 dark:border-neutral-700 p-4 w-60 h-28 flex flex-col justify-between">
      <div className="flex items-center gap-2 text-xs text-gray-600 dark:text-neutral-300">
        <Icon size={16} className="text-accent-blue" />
        <span className="font-medium">{title}</span>
      </div>
      <div className="text-2xl font-semibold">{big}</div>
      <div className="text-[10px] text-gray-500 dark:text-neutral-400">{sub}</div>
    </div>
  );
}

function Section({
  title,
  children,
}: {
  title: string;
  children: React.ReactNode;
}) {
  return (
    <section className="bg-white dark:bg-neutral-800 rounded-lg border border-gray-200 dark:border-neutral-700">
      <h2 className="px-4 py-3 text-sm font-medium border-b border-gray-200 dark:border-neutral-700">
        {title}
      </h2>
      <div className="p-4">{children}</div>
    </section>
  );
}

function Table({
  headers,
  empty,
  children,
}: {
  headers: string[];
  empty: string | null;
  children?: React.ReactNode;
}) {
  return (
    <table className="w-full text-xs">
      <thead>
        <tr className="text-[10px] uppercase tracking-wider text-gray-500 dark:text-neutral-400">
          {headers.map((h) => (
            <th
              key={h}
              className="text-left font-medium pb-2 pr-4"
              style={{ width: h === "Log length" ? 120 : undefined }}
            >
              {h}
            </th>
          ))}
        </tr>
      </thead>
      <tbody>
        {children}
        {empty && (
          <tr>
            <td
              colSpan={headers.length}
              className="py-4 text-gray-400 dark:text-neutral-500 text-center"
            >
              {empty}
            </td>
          </tr>
        )}
      </tbody>
    </table>
  );
}