import { useQuery } from "@tanstack/react-query";
import {
  Server,
  Network,
  AlertTriangle,
  ChevronRight,
  Workflow,
  Activity,
  type LucideIcon,
} from "lucide-react";
import { useStore } from "../state/store";
import {
  clusterStatus,
  listJobs,
  type ClusterMetrics,
  type JobSummary,
} from "../ipc";

const REFRESH_MS = 5000;

// Compass-inspired card grid: each "thing" (node, job) is a card with
// metadata + a colored tag. The 3 stat cards on top become compact
// counters in the toolbar.
export function Dashboard() {
  const addr = useStore((s) => s.addr);
  eprintln!("[bee-gui Dashboard] render: addr={}", addr);
  const clusterQ = useQuery<ClusterMetrics>({
    queryKey: ["cluster", addr],
    queryFn: () => {
      eprintln!("[bee-gui Dashboard] cluster_status fired with addr={}", addr);
      return clusterStatus(addr);
    },
    refetchInterval: REFRESH_MS,
  });
  const jobsQ = useQuery<JobSummary[]>({
    queryKey: ["jobs", addr],
    queryFn: () => {
      eprintln!("[bee-gui Dashboard] list_jobs fired with addr={}", addr);
      return listJobs(addr);
    },
    refetchInterval: REFRESH_MS,
  });

  const refreshing = clusterQ.isFetching || jobsQ.isFetching;

  return (
    <div className="space-y-6">
      {(clusterQ.error || jobsQ.error) && (
        <div className="flex items-center gap-2 px-3 py-2 text-sm rounded-md bg-red-50 dark:bg-red-900/20 text-accent-red border border-red-200 dark:border-red-800">
          <AlertTriangle size={14} />
          RPC error: {String(clusterQ.error || jobsQ.error)}
        </div>
      )}

      {/* Stat counters — compact, inline */}
      <div className="grid grid-cols-3 gap-4">
        <StatCard
          Icon={Server}
          label="Cluster"
          value={clusterQ.data?.nodes.length ?? 0}
          sub={
            clusterQ.data
              ? `${clusterQ.data.term} · commit ${clusterQ.data.commit_index}`
              : clusterQ.isLoading
                ? "loading…"
                : "no data"
          }
        />
        <StatCard
          Icon={Workflow}
          label="Pipelines"
          value={jobsQ.data?.length ?? 0}
          sub={`${jobsQ.data?.filter((j) => j.lifecycle === "Running").length ?? 0} running`}
        />
        <StatCard
          Icon={Activity}
          label="Tasks"
          value={jobsQ.data?.reduce((s, j) => s + j.task_count, 0) ?? 0}
          sub={`${jobsQ.data?.filter((j) => j.lifecycle === "Completed").length ?? 0} completed`}
        />
      </div>

      {/* Node cards */}
      <CardSection
        title={`Nodes (${clusterQ.data?.nodes.length ?? 0})`}
        empty={clusterQ.isLoading ? "loading…" : "click Refresh"}
        emptyIcon={Network}
      >
        {clusterQ.data?.nodes.map((n) => (
          <NodeCard
            key={n.id}
            id={n.id}
            role={n.role}
            commit={n.commit_index}
            logLength={n.log_length}
            leaderId={clusterQ.data.leader_id}
            term={clusterQ.data.term}
          />
        ))}
      </CardSection>

      {/* Job cards */}
      <CardSection
        title={`Recent Jobs (${jobsQ.data?.length ?? 0})`}
        empty={jobsQ.isLoading ? "loading…" : "no jobs — click Refresh"}
        emptyIcon={Workflow}
      >
        {jobsQ.data?.map((j) => (
          <JobCard
            key={j.job_id}
            id={j.job_id}
            dagHash={j.dag_hash}
            lifecycle={j.lifecycle}
            mode={j.mode}
            taskCount={j.task_count}
            ownerNode={j.owner_node}
          />
        ))}
      </CardSection>

      {refreshing && (
        <p className="text-xs text-gray-400 text-center">refreshing…</p>
      )}
    </div>
  );
}

function StatCard({
  Icon,
  label,
  value,
  sub,
}: {
  Icon: LucideIcon;
  label: string;
  value: number;
  sub: string;
}) {
  return (
    <div className="bg-white dark:bg-neutral-800 rounded-lg border border-gray-200 dark:border-neutral-700 p-4 flex items-center gap-3">
      <div className="w-10 h-10 rounded-md bg-accent-blue/10 text-accent-blue flex items-center justify-center">
        <Icon size={20} />
      </div>
      <div className="flex-1">
        <div className="text-xs text-gray-500 dark:text-neutral-400">{label}</div>
        <div className="text-2xl font-semibold tabular-nums">{value}</div>
        <div className="text-[10px] text-gray-400">{sub}</div>
      </div>
    </div>
  );
}

function CardSection({
  title,
  empty,
  emptyIcon: EmptyIcon,
  children,
}: {
  title: string;
  empty?: string;
  emptyIcon?: LucideIcon;
  children?: React.ReactNode;
}) {
  const arr = Array.isArray(children) ? children : children ? [children] : [];
  return (
    <section>
      <h2 className="text-sm font-semibold text-gray-700 dark:text-neutral-200 mb-2 px-1">
        {title}
      </h2>
      {arr.length === 0 ? (
        <div className="bg-white dark:bg-neutral-800 rounded-lg border border-gray-200 dark:border-neutral-700 border-dashed p-8 text-center">
          {EmptyIcon && (
            <EmptyIcon
              size={32}
              className="mx-auto text-gray-300 dark:text-neutral-600"
            />
          )}
          <p className="mt-2 text-xs text-gray-400">{empty}</p>
        </div>
      ) : (
        <div className="grid grid-cols-1 md:grid-cols-2 lg:grid-cols-3 xl:grid-cols-4 gap-3">
          {children}
        </div>
      )}
    </section>
  );
}

const roleColor = (role: string) =>
  role === "Leader"
    ? "bg-accent-blue text-white"
    : role === "Candidate"
      ? "bg-accent-orange text-white"
      : "bg-gray-500 text-white";

const lifeColor = (s: string) =>
  s === "Running"
    ? "bg-accent-green text-white"
    : s === "Completed"
      ? "bg-accent-blue text-white"
      : s === "Failed"
        ? "bg-accent-red text-white"
        : s === "WaitingForUpstream"
          ? "bg-accent-orange text-white"
          : "bg-gray-400 text-white";

function NodeCard({
  id,
  role,
  commit,
  logLength,
  leaderId,
  term,
}: {
  id: number;
  role: string;
  commit: number;
  logLength: number;
  leaderId: number | null;
  term: number;
}) {
  const isLeader = role === "Leader";
  return (
    <article className="bg-white dark:bg-neutral-800 rounded-lg border border-gray-200 dark:border-neutral-700 p-3 hover:shadow-md transition-shadow">
      <div className="flex items-start justify-between">
        <div className="flex items-center gap-1.5">
          <Server size={14} className="text-gray-500 dark:text-neutral-400" />
          <span className="text-sm font-medium">node #{id}</span>
        </div>
        <span
          className={`px-1.5 py-0.5 rounded text-[10px] font-semibold uppercase ${roleColor(role)}`}
        >
          {role}
        </span>
      </div>
      <dl className="mt-2 space-y-0.5 text-[11px] text-gray-600 dark:text-neutral-400">
        <div className="flex justify-between">
          <dt>commit</dt>
          <dd className="font-mono">{commit}</dd>
        </div>
        <div className="flex justify-between">
          <dt>log length</dt>
          <dd className="font-mono">{logLength}</dd>
        </div>
        <div className="flex justify-between">
          <dt>term</dt>
          <dd className="font-mono">{term}</dd>
        </div>
        {isLeader && (
          <div className="flex justify-between text-accent-blue">
            <dt>leader</dt>
            <dd className="font-mono">self</dd>
          </div>
        )}
        {!isLeader && leaderId && (
          <div className="flex justify-between text-gray-500">
            <dt>leader</dt>
            <dd className="font-mono">#{leaderId}</dd>
          </div>
        )}
      </dl>
      <button className="mt-2 w-full flex items-center justify-center gap-1 py-1 text-[10px] text-accent-blue hover:bg-accent-blue/5 rounded">
        Inspect <ChevronRight size={10} />
      </button>
    </article>
  );
}

function JobCard({
  id,
  dagHash,
  lifecycle,
  mode,
  taskCount,
  ownerNode,
}: {
  id: number;
  dagHash: string;
  lifecycle: string;
  mode: string;
  taskCount: number;
  ownerNode: number;
}) {
  return (
    <article className="bg-white dark:bg-neutral-800 rounded-lg border border-gray-200 dark:border-neutral-700 p-3 hover:shadow-md transition-shadow">
      <div className="flex items-start justify-between">
        <div className="flex items-center gap-1.5 min-w-0">
          <Workflow size={14} className="text-gray-500 dark:text-neutral-400 shrink-0" />
          <span className="text-sm font-medium truncate">#{id}</span>
        </div>
        <span
          className={`px-1.5 py-0.5 rounded text-[10px] font-semibold uppercase ${lifeColor(lifecycle)}`}
        >
          {lifecycle}
        </span>
      </div>
      <div className="mt-2 text-[10px] font-mono text-gray-500 dark:text-neutral-400 truncate">
        {dagHash}
      </div>
      <dl className="mt-2 space-y-0.5 text-[11px] text-gray-600 dark:text-neutral-400">
        <div className="flex justify-between">
          <dt>mode</dt>
          <dd>{mode}</dd>
        </div>
        <div className="flex justify-between">
          <dt>tasks</dt>
          <dd className="font-mono">{taskCount}</dd>
        </div>
        <div className="flex justify-between">
          <dt>owner</dt>
          <dd className="font-mono">#{ownerNode}</dd>
        </div>
      </dl>
      <button className="mt-2 w-full flex items-center justify-center gap-1 py-1 text-[10px] text-accent-blue hover:bg-accent-blue/5 rounded">
        Inspect <ChevronRight size={10} />
      </button>
    </article>
  );
}