import { useEffect, useMemo, useRef, useState } from "react";
import { useQuery } from "@tanstack/react-query";
import { X, AlertTriangle, ChevronRight, ArrowLeft, Plus } from "lucide-react";

import { useConnection } from "../state/connectionStore";
import { useTabs } from "../state/tabsStore";
import {
  pipelineGet,
  pipelineList,
  jobInspect,
  datasourceList,
  type PipelineDefinitionView,
  type DatasourceView,
  type JobDetail,
} from "../ipc";
import {
  PipelineGraph,
  parsePipeline,
  type PipelineDefinition,
  type CrossPipelineRef,
  type HandlerRef,
} from "../domain/pipeline";

type Mode = "definition" | "runtime";

const LOADING_TIMEOUT_MS = 5000;

export function PipelineDetail({ pipelineId }: { pipelineId: number }) {
  const [mode, setMode] = useState<Mode>("definition");
  const [handlerDrawer, setHandlerDrawer] = useState<HandlerRef | null>(null);
  const [datasourceDrawer, setDatasourceDrawer] = useState<
    | { kind: "input"; missing: boolean; name?: string }
    | { kind: "output"; missing: boolean; name?: string }
    | null
  >(null);

  const addr = useConnection((s) => s.addr);
  const openTab = useTabs((s) => s.open);
  const closeTab = useTabs((s) => s.close);
  const currentTabId = useTabs((s) =>
    s.tabs.find((t) => t.kind === "pipeline" && t.resource_id === String(pipelineId))?.id ?? null,
  );

  const isInvalidId = !Number.isFinite(pipelineId);

  const q = useQuery<PipelineDefinitionView | null>({
    queryKey: ["pipeline", pipelineId],
    queryFn: () => pipelineGet(pipelineId),
    enabled: !isInvalidId,
    retry: false,
  });

  const pipeline: PipelineDefinition | null = useMemo(() => {
    if (!q.data) return null;
    return parsePipeline(q.data);
  }, [q.data]);

  const hadDataRef = useRef(false);
  useEffect(() => {
    if (q.data) {
      hadDataRef.current = true;
      return;
    }
    if (hadDataRef.current && !q.isLoading && !q.error && currentTabId !== null) {
      void closeTab(currentTabId);
    }
  }, [q.data, q.isLoading, q.error, currentTabId, closeTab]);

  const [timedOut, setTimedOut] = useState(false);
  useEffect(() => {
    if (!q.isLoading) {
      setTimedOut(false);
      return;
    }
    const t = window.setTimeout(() => setTimedOut(true), LOADING_TIMEOUT_MS);
    return () => window.clearTimeout(t);
  }, [q.isLoading]);

  if (isInvalidId) {
    return (
      <p
        className="text-xs text-gray-400"
        data-testid="pipeline-invalid-id"
      >
        invalid pipeline id
      </p>
    );
  }

  if (q.isFetching && !q.data && !q.error && !timedOut) {
    return <p className="text-xs text-gray-500">loading…</p>;
  }
  if (q.error || timedOut) {
    const message = timedOut
      ? `Pipeline load timed out after ${LOADING_TIMEOUT_MS / 1000}s`
      : `Failed to load pipeline: ${String(q.error)}`;
    return (
      <div
        role="alert"
        className="flex items-center gap-2 text-sm rounded-md bg-red-50 dark:bg-red-900/20 text-accent-red border border-red-200 dark:border-red-800 p-3"
      >
        <AlertTriangle size={14} />
        <span className="flex-1">{message}</span>
        <button
          type="button"
          onClick={() => {
            void q.refetch();
          }}
          className="px-2 py-1 text-xs rounded border border-red-200 dark:border-red-800 hover:bg-red-100 dark:hover:bg-red-900/30"
        >
          Retry
        </button>
      </div>
    );
  }
  if (!pipeline) {
    const onBackToList = async () => {
      if (currentTabId !== null) {
        await closeTab(currentTabId);
      }
      await openTab({
        kind: "application_pipelines",
        resourceId: null,
        title: "Pipelines",
      });
    };
    const onCreateNew = () =>
      openTab({
        kind: "pipeline_editor",
        resourceId: null,
        title: "New Pipeline",
      });
    return (
      <div
        role="alert"
        className="max-w-md rounded-lg border border-gray-200 dark:border-neutral-700 bg-white dark:bg-neutral-800 p-5 space-y-3"
        data-testid="pipeline-not-found"
      >
        <h2 className="text-sm font-semibold">Pipeline #{pipelineId} not found</h2>
        <p className="text-xs text-gray-500 dark:text-neutral-400">
          It may have been deleted, or the database was reset.
        </p>
        <div className="flex items-center gap-2 pt-1">
          <button
            type="button"
            onClick={() => {
              void onBackToList();
            }}
            className="inline-flex items-center gap-1.5 px-3 py-1.5 text-xs rounded-md bg-accent-blue text-white hover:bg-accent-blue/90"
          >
            <ArrowLeft size={14} />
            Back to Pipelines list
          </button>
          <button
            type="button"
            onClick={() => {
              void onCreateNew();
            }}
            className="inline-flex items-center gap-1.5 px-3 py-1.5 text-xs rounded-md border border-gray-200 dark:border-neutral-700 text-gray-700 dark:text-neutral-200 hover:bg-gray-100 dark:hover:bg-neutral-700"
          >
            <Plus size={14} />
            Create new pipeline
          </button>
        </div>
      </div>
    );
  }

  const handleCrossPipelineRef = async (ref: CrossPipelineRef) => {
    const list = await pipelineList();
    const target = list.find((p) => p.name === ref.upstreamPipelineName);
    if (!target) return;
    await openTab({
      kind: "pipeline",
      resourceId: String(target.id),
      title: target.name,
    });
  };

  return (
    <div className="space-y-4">
      <header className="flex items-center justify-between">
        <div>
          <h1 className="text-xl font-semibold">{pipeline.name}</h1>
          <p className="text-xs text-gray-500 dark:text-neutral-400 font-mono">#{pipeline.id}</p>
        </div>
        <div className="inline-flex items-center rounded-md border border-gray-200 dark:border-neutral-700 overflow-hidden">
          <button
            type="button"
            onClick={() => setMode("definition")}
            className={[
              "px-3 py-1 text-xs",
              mode === "definition"
                ? "bg-accent-blue text-white"
                : "bg-white dark:bg-neutral-800 text-gray-600 dark:text-neutral-300",
            ].join(" ")}
          >
            Definition
          </button>
          <button
            type="button"
            onClick={() => setMode("runtime")}
            className={[
              "px-3 py-1 text-xs",
              mode === "runtime"
                ? "bg-accent-blue text-white"
                : "bg-white dark:bg-neutral-800 text-gray-600 dark:text-neutral-300",
            ].join(" ")}
          >
            Runtime
          </button>
        </div>
      </header>

      {mode === "definition" && (
        <section className="bg-white dark:bg-neutral-800 rounded-lg border border-gray-200 dark:border-neutral-700 p-4">
          <PipelineGraph
            pipeline={pipeline}
            onSelectInput={() =>
              setDatasourceDrawer({ kind: "input", missing: false, name: pipeline.input.datasource })
            }
            onSelectOutput={() =>
              setDatasourceDrawer({
                kind: "output",
                missing: false,
                name: pipeline.output.adapter,
              })
            }
            onSelectHandler={(id) => {
              const found = pipeline.handlers.find((h) => h.id === id) ?? null;
              setHandlerDrawer(found);
            }}
            onSelectCrossPipelineRef={(ref) => {
              void handleCrossPipelineRef(ref);
            }}
          />
        </section>
      )}

      {mode === "runtime" && (
        <RuntimeView pipelineId={pipelineId} addr={addr} />
      )}

      {datasourceDrawer && (
        <DatasourceDrawer
          kind={datasourceDrawer.kind}
          referenceName={datasourceDrawer.name}
          onClose={() => setDatasourceDrawer(null)}
        />
      )}
      {handlerDrawer && (
        <HandlerDrawer handler={handlerDrawer} onClose={() => setHandlerDrawer(null)} />
      )}
    </div>
  );
}

function RuntimeView({ pipelineId, addr }: { pipelineId: number; addr: string }) {
  const rq = useQuery<JobDetail | null>({
    queryKey: ["job", addr, pipelineId],
    queryFn: () => jobInspect(addr, pipelineId),
  });
  if (rq.isLoading) {
    return <p className="text-xs text-gray-500">loading…</p>;
  }
  if (!rq.data) {
    return <p className="text-xs text-gray-500">no runtime data — job not running</p>;
  }
  const j = rq.data;
  return (
    <section className="bg-white dark:bg-neutral-800 rounded-lg border border-gray-200 dark:border-neutral-700 p-4">
      <h2 className="text-sm font-semibold mb-3">Job #{j.job_id}</h2>
      <dl className="grid grid-cols-2 gap-2 text-xs">
        <Pair label="lifecycle" value={j.lifecycle} />
        <Pair label="owner_node" value={`#${j.owner_node}`} />
        <Pair label="dag_hash" value={j.dag_hash} />
        <Pair label="tasks" value={String(j.tasks.length)} />
      </dl>
      <h3 className="text-xs font-medium mt-4 mb-2">Tasks</h3>
      <ul className="text-[11px] space-y-1 font-mono">
        {j.tasks.map((t) => (
          <li key={t.task_id} className="flex items-center gap-2">
            <ChevronRight size={10} className="text-gray-400" />
            <span>task #{t.task_id} phase {t.phase_id} owner #{t.owner_node} status {t.status}</span>
          </li>
        ))}
      </ul>
    </section>
  );
}

function Pair({ label, value }: { label: string; value: string }) {
  return (
    <div className="flex items-center gap-2">
      <dt className="text-gray-500 dark:text-neutral-400 w-24">{label}</dt>
      <dd className="font-mono">{value}</dd>
    </div>
  );
}

function HandlerDrawer({
  handler,
  onClose,
}: {
  handler: HandlerRef;
  onClose: () => void;
}) {
  useEscape(onClose);
  return (
    <div
      role="dialog"
      aria-label="handler details"
      className="fixed inset-y-0 right-0 z-40 w-80 bg-white dark:bg-neutral-800 border-l border-gray-200 dark:border-neutral-700 shadow-xl flex flex-col"
    >
      <header className="flex items-center justify-between px-4 py-3 border-b border-gray-200 dark:border-neutral-700">
        <h2 className="text-sm font-semibold">Phase · {handler.id}</h2>
        <button
          type="button"
          onClick={onClose}
          aria-label="close handler drawer"
          className="p-1 rounded text-gray-500 hover:bg-gray-100 dark:hover:bg-neutral-700"
        >
          <X size={14} />
        </button>
      </header>
      <div className="p-4 space-y-3 overflow-y-auto">
        <div>
          <div className="text-[10px] uppercase tracking-wider text-gray-500 dark:text-neutral-400">
            Handler
          </div>
          <div className="text-sm font-mono">{handler.name}</div>
        </div>
        <div>
          <div className="text-[10px] uppercase tracking-wider text-gray-500 dark:text-neutral-400">
            Parameters
          </div>
          {Object.keys(handler.params).length === 0 ? (
            <p className="text-xs text-gray-400">no parameters</p>
          ) : (
            <dl className="text-xs space-y-1 mt-1">
              {Object.entries(handler.params).map(([k, v]) => (
                <div key={k} className="flex items-center gap-2">
                  <dt className="text-gray-500 dark:text-neutral-400 w-24">{k}</dt>
                  <dd className="font-mono">{String(v)}</dd>
                </div>
              ))}
            </dl>
          )}
        </div>
        <div>
          <div className="text-[10px] uppercase tracking-wider text-gray-500 dark:text-neutral-400">
            Upstream
          </div>
          {handler.upstream.length === 0 ? (
            <p className="text-xs text-gray-400">none</p>
          ) : (
            <ul className="text-xs font-mono mt-1">
              {handler.upstream.map((u) => (
                <li key={u}>↑ {u}</li>
              ))}
            </ul>
          )}
        </div>
      </div>
    </div>
  );
}

function DatasourceDrawer({
  kind,
  referenceName,
  onClose,
}: {
  kind: "input" | "output";
  referenceName?: string;
  onClose: () => void;
}) {
  useEscape(onClose);
  const q = useQuery<DatasourceView[]>({
    queryKey: ["datasources-local"],
    queryFn: () => datasourceList(),
  });
  const list = q.data ?? [];
  const match = referenceName ? list.find((d) => d.name === referenceName) : undefined;

  return (
    <div
      role="dialog"
      aria-label="datasource details"
      className="fixed inset-y-0 right-0 z-40 w-80 bg-white dark:bg-neutral-800 border-l border-gray-200 dark:border-neutral-700 shadow-xl flex flex-col"
    >
      <header className="flex items-center justify-between px-4 py-3 border-b border-gray-200 dark:border-neutral-700">
        <h2 className="text-sm font-semibold">
          {kind === "input" ? "Input Datasource" : "Output Adapter"}
        </h2>
        <button
          type="button"
          onClick={onClose}
          aria-label="close datasource drawer"
          className="p-1 rounded text-gray-500 hover:bg-gray-100 dark:hover:bg-neutral-700"
        >
          <X size={14} />
        </button>
      </header>
      <div className="p-4 text-xs space-y-3 overflow-y-auto">
        {referenceName && (
          <div>
            <div className="text-[10px] uppercase tracking-wider text-gray-500 dark:text-neutral-400">
              Reference
            </div>
            <div className="font-mono">{referenceName}</div>
          </div>
        )}
        {q.isLoading && <p className="text-gray-500">loading…</p>}
        {!q.isLoading && !match && (
          <p className="text-gray-500">not configured — no matching datasource registered</p>
        )}
        {match && (
          <div className="space-y-2">
            <Pair label="name" value={match.name} />
            <Pair label="plugin" value={match.plugin} />
            <Pair label="tenant" value={String(match.tenant)} />
            <Pair label="config" value={match.config} />
          </div>
        )}
      </div>
    </div>
  );
}

function useEscape(onClose: () => void) {
  useEffect(() => {
    const onKey = (e: KeyboardEvent) => {
      if (e.key === "Escape") onClose();
    };
    window.addEventListener("keydown", onKey);
    return () => window.removeEventListener("keydown", onKey);
  }, [onClose]);
}
