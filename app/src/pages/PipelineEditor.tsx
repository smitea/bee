import { useCallback, useEffect, useMemo, useRef, useState } from "react";
import CodeMirror from "@uiw/react-codemirror";
import { sql } from "@codemirror/lang-sql";
import { oneDark } from "@codemirror/theme-one-dark";
import {
  ReactFlow,
  Background,
  Controls,
  MiniMap,
  addEdge,
  applyEdgeChanges,
  applyNodeChanges,
  type Connection,
  type Edge,
  type EdgeChange,
  type Node,
  type NodeChange,
} from "@xyflow/react";
import "@xyflow/react/dist/style.css";
import { Save, X, Play, Download, AlertTriangle } from "lucide-react";
import { useQueryClient } from "@tanstack/react-query";

import { useUi } from "../state/store";
import { useTabs } from "../state/tabsStore";
import { useNavigation } from "../state/navigationStore";
import { pipelineCreate, pipelineGet, pipelineDelete, type PipelineDefinitionView } from "../ipc/pipelines";
import { pipelineDumpRecord } from "../ipc/pipeline_dumps";
import { jobInspect } from "../ipc/cluster";
import { useConnection } from "../state/connectionStore";

interface PipelineDag {
  input?: {
    datasource?: string;
    method?: string;
    args?: Record<string, unknown>;
    output?: string;
  };
  handlers?: Array<{
    id: string;
    name: string;
    params?: Record<string, unknown>;
    upstream?: string[];
  }>;
  output?: {
    adapter?: string;
    method?: string;
    args?: Record<string, unknown>;
    upstream?: string;
  };
  crossPipelineRefs?: unknown[];
}

interface Props {
  pipelineId?: number;
}

const DEFAULT_DAG: PipelineDag = {
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
};

const SAMPLE_SQL = `-- Define a Bee pipeline SQL view of the DAG
-- Each handler reads its upstream stream and produces one output.
SELECT 'binance.subscribe' AS input, 'in' AS into;
SELECT 'indicator_ema' AS handler, ARRAY['in'] AS upstream, 'out' AS into;
`;

function dagToNodesEdges(dag: PipelineDag, fallback: string): { nodes: Node[]; edges: Edge[] } {
  const nodes: Node[] = [];
  const edges: Edge[] = [];
  const inId = dag.input?.output ?? "in";
  nodes.push({
    id: "input",
    type: "default",
    position: { x: 40, y: 40 },
    data: {
      label: `Input · ${dag.input?.datasource ?? "?"}.${dag.input?.method ?? "?"}`,
    },
    style: { background: "#dbeafe", border: "1px solid #3b82f6", borderRadius: 4 },
  });
  let y = 120;
  for (const h of dag.handlers ?? []) {
    nodes.push({
      id: h.id,
      type: "default",
      position: { x: 240, y },
      data: { label: `Handler · ${h.id}\n${h.name}` },
      style: { background: "#fff", border: "1px solid #9ca3af", borderRadius: 4 },
    });
    for (const up of h.upstream ?? []) {
      edges.push({ id: `${up}->${h.id}`, source: up, target: h.id });
    }
    y += 80;
  }
  const outId = `output-${fallback}`;
  nodes.push({
    id: outId,
    type: "default",
    position: { x: 480, y: 40 },
    data: { label: `Output · ${dag.output?.adapter ?? "?"}.${dag.output?.method ?? "?"}` },
    style: { background: "#dcfce7", border: "1px solid #22c55e", borderRadius: 4 },
  });
  const lastUp = dag.output?.upstream ?? inId;
  edges.push({ id: `${lastUp}->${outId}`, source: lastUp, target: outId });
  return { nodes, edges };
}

export function PipelineEditor({ pipelineId }: Props = {}) {
  const openTab = useTabs((s) => s.open);
  const close = useTabs((s) => s.close);
  const theme = useUi((s) => s.theme);
  const nav = useNavigation();
  const qc = useQueryClient();
  const addr = useConnection((s) => s.addr);

  const editorTabId = useTabs((s) => {
    const active = s.tabs.find((t) => t.id === s.activeId);
    return active && active.kind === "pipeline_editor" ? active.id : null;
  });

  const [name, setName] = useState("");
  const [dagJson, setDagJson] = useState(JSON.stringify(DEFAULT_DAG, null, 2));
  const [sqlText, setSqlText] = useState(SAMPLE_SQL);
  const [tab, setTab] = useState<"sql" | "dag">("dag");
  const [error, setError] = useState<string | null>(null);
  const [busy, setBusy] = useState(false);
  const [debugInfo, setDebugInfo] = useState<string | null>(null);
  const fileInputRef = useRef<HTMLInputElement | null>(null);

  useEffect(() => {
    if (pipelineId === undefined) return;
    let cancelled = false;
    void pipelineGet(pipelineId).then((p: PipelineDefinitionView | null) => {
      if (cancelled || !p) return;
      setName(p.name);
      setDagJson(p.dag_json);
      try {
        const parsed = JSON.parse(p.dag_json);
        setSqlText(buildSqlFromDag(parsed));
      } catch {}
    });
    return () => {
      cancelled = true;
    };
  }, [pipelineId]);

  const parsed: PipelineDag | null = useMemo(() => {
    try {
      return JSON.parse(dagJson) as PipelineDag;
    } catch {
      return null;
    }
  }, [dagJson]);

  const initial = useMemo(() => dagToNodesEdges(parsed ?? DEFAULT_DAG, "1"), [parsed]);
  const [nodes, setNodes] = useState<Node[]>(initial.nodes);
  const [edges, setEdges] = useState<Edge[]>(initial.edges);
  const initializedRef = useRef(false);
  useEffect(() => {
    if (!initializedRef.current) {
      initializedRef.current = true;
      return;
    }
    const next = dagToNodesEdges(parsed ?? DEFAULT_DAG, "1");
    setNodes(next.nodes);
    setEdges(next.edges);
  }, [parsed]);

  const onNodesChange = useCallback(
    (changes: NodeChange[]) => setNodes((nds) => applyNodeChanges(changes, nds)),
    [],
  );
  const onEdgesChange = useCallback(
    (changes: EdgeChange[]) => setEdges((eds) => applyEdgeChanges(changes, eds)),
    [],
  );
  const onConnect = useCallback(
    (c: Connection) =>
      setEdges((eds) => addEdge({ ...c, id: `${c.source}->${c.target}` }, eds)),
    [],
  );

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
      if (pipelineId !== undefined) {
        await pipelineDelete(pipelineId).catch(() => {});
      }
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

  const onCancel = () => {
    if (editorTabId !== null) {
      void close(editorTabId);
      return;
    }
    nav.back();
  };

  const onDebug = async () => {
    setError(null);
    setDebugInfo("running synthetic tick…");
    try {
      let dag: PipelineDag;
      try {
        dag = JSON.parse(dagJson) as PipelineDag;
      } catch {
        setError("invalid JSON in dag_json");
        return;
      }
      const trimmed = name.trim() || `debug-${Date.now()}`;
      const created = await pipelineCreate(trimmed, JSON.stringify(dag));
      setDebugInfo(`synthetic pipeline created as #${created.id}`);
      try {
        const detail = await jobInspect(addr, created.id);
        if (detail) {
          setDebugInfo(
            `pipeline #${created.id} lifecycle=${detail.lifecycle} tasks=${detail.tasks.length}`,
          );
        }
      } catch {
        /* job inspect may fail if not yet running */
      }
      void qc.invalidateQueries({ queryKey: ["jobs", addr] });
      void qc.invalidateQueries({ queryKey: ["pipeline-defs"] });
    } catch (e) {
      setError(String(e));
      setDebugInfo(null);
    }
  };

  const onDump = async () => {
    setError(null);
    try {
      const payload = JSON.stringify({
        name,
        dag: JSON.parse(dagJson),
        sql: sqlText,
        dumped_at: Math.floor(Date.now() / 1000),
      });
      const dumpId = pipelineId ?? 0;
      await pipelineDumpRecord(dumpId, payload);
      const blob = new Blob([payload], { type: "application/json" });
      const url = URL.createObjectURL(blob);
      const a = document.createElement("a");
      a.href = url;
      a.download = `${name || "pipeline"}.json`;
      a.click();
      URL.revokeObjectURL(url);
      setDebugInfo(`dump recorded and downloaded`);
    } catch (e) {
      setError(String(e));
    }
  };

  return (
    <div className="space-y-4">
      <header className="flex items-center justify-between">
        <div>
          <h1 className="text-xl font-semibold">
            {pipelineId !== undefined ? `Edit Pipeline #${pipelineId}` : "New Pipeline"}
          </h1>
          <p className="text-xs text-gray-500 dark:text-neutral-400">
            SQL view · DAG view · Debug · Dump — two-way synced
          </p>
        </div>
        <div className="flex items-center gap-2">
          <button
            type="button"
            onClick={onCancel}
            className="flex items-center gap-1 px-3 py-1.5 text-xs rounded border border-gray-200 dark:border-neutral-700 hover:bg-gray-50 dark:hover:bg-neutral-700"
            data-testid="pipeline-cancel"
          >
            <X size={12} />
            Cancel
          </button>
          <button
            type="button"
            onClick={() => void onDebug()}
            className="flex items-center gap-1 px-3 py-1.5 text-xs rounded border border-gray-200 dark:border-neutral-700 hover:bg-gray-50 dark:hover:bg-neutral-700"
            data-testid="pipeline-debug"
          >
            <Play size={12} />
            Debug
          </button>
          <button
            type="button"
            onClick={() => void onDump()}
            className="flex items-center gap-1 px-3 py-1.5 text-xs rounded border border-gray-200 dark:border-neutral-700 hover:bg-gray-50 dark:hover:bg-neutral-700"
            data-testid="pipeline-dump"
          >
            <Download size={12} />
            Dump
          </button>
          <button
            type="button"
            onClick={() => void onSave()}
            disabled={busy}
            className="flex items-center gap-1 px-3 py-1.5 text-xs rounded bg-accent-blue text-white hover:bg-accent-blue/90 disabled:opacity-50"
            data-testid="pipeline-save"
          >
            <Save size={12} />
            Save pipeline
          </button>
        </div>
      </header>

      <input
        ref={fileInputRef}
        type="file"
        accept="application/json"
        className="hidden"
      />

      <div className="grid grid-cols-1 lg:grid-cols-[1fr_2fr] gap-4">
        <section className="bg-white dark:bg-neutral-800 rounded-lg border border-gray-200 dark:border-neutral-700 p-4 space-y-3">
          <label className="flex flex-col gap-1">
            <span className="text-[10px] text-gray-500 dark:text-neutral-400">Name</span>
            <input
              placeholder="pipeline name"
              value={name}
              onChange={(e) => setName(e.target.value)}
              data-testid="pipeline-name"
              className="px-2 py-1 text-xs rounded border border-gray-200 dark:border-neutral-700 bg-white dark:bg-neutral-900"
            />
          </label>

          <div className="flex items-center gap-2 text-[10px]">
            <button
              type="button"
              onClick={() => setTab("dag")}
              data-testid="tab-dag"
              className={[
                "px-2 py-1 rounded",
                tab === "dag"
                  ? "bg-accent-blue/15 text-accent-blue"
                  : "text-gray-500 dark:text-neutral-400 hover:bg-gray-100 dark:hover:bg-neutral-700",
              ].join(" ")}
            >
              DAG (JSON)
            </button>
            <button
              type="button"
              onClick={() => setTab("sql")}
              data-testid="tab-sql"
              className={[
                "px-2 py-1 rounded",
                tab === "sql"
                  ? "bg-accent-blue/15 text-accent-blue"
                  : "text-gray-500 dark:text-neutral-400 hover:bg-gray-100 dark:hover:bg-neutral-700",
              ].join(" ")}
            >
              SQL
            </button>
          </div>

          {tab === "dag" ? (
            <CodeMirror
              value={dagJson}
              height="380px"
              theme={theme === "dark" ? oneDark : "light"}
              extensions={[sql()]}
              onChange={(v) => {
                setDagJson(v);
                try {
                  const parsedDag = JSON.parse(v) as PipelineDag;
                  setSqlText(buildSqlFromDag(parsedDag));
                } catch {}
              }}
            />
          ) : (
            <CodeMirror
              value={sqlText}
              height="380px"
              theme={theme === "dark" ? oneDark : "light"}
              extensions={[sql()]}
              onChange={(v) => setSqlText(v)}
            />
          )}
          {error && (
            <div
              role="alert"
              className="flex items-center gap-2 text-xs rounded-md bg-red-50 dark:bg-red-900/20 text-accent-red border border-red-200 dark:border-red-800 p-2"
            >
              <AlertTriangle size={12} />
              {error}
            </div>
          )}
          {debugInfo && (
            <div className="text-[10px] text-gray-500 dark:text-neutral-400 font-mono">
              {debugInfo}
            </div>
          )}
        </section>

        <section
          aria-label="dag designer"
          className="bg-white dark:bg-neutral-800 rounded-lg border border-gray-200 dark:border-neutral-700"
          style={{ height: 460 }}
          data-testid="dag-designer"
        >
          <ReactFlow
            nodes={nodes}
            edges={edges}
            onNodesChange={onNodesChange}
            onEdgesChange={onEdgesChange}
            onConnect={onConnect}
            fitView
          >
            <Background gap={20} />
            <Controls />
            <MiniMap pannable zoomable />
          </ReactFlow>
        </section>
      </div>
    </div>
  );
}

function buildSqlFromDag(dag: PipelineDag): string {
  const lines: string[] = [];
  const inId = dag.input?.output ?? "in";
  lines.push(
    `-- Input: ${dag.input?.datasource ?? "?"}.${dag.input?.method ?? "?"} → ${inId}`,
  );
  for (const h of dag.handlers ?? []) {
    lines.push(
      `-- Handler: ${h.id} ${h.name} ← [${(h.upstream ?? []).join(", ")}]`,
    );
  }
  lines.push(
    `-- Output: ${dag.output?.adapter ?? "?"}.${dag.output?.method ?? "?"} ← ${dag.output?.upstream ?? inId}`,
  );
  return lines.join("\n");
}

export { PipelineEditor as PipelineDesigner };