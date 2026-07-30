import { useEffect, useRef, useState } from "react";
import { useQuery, useQueryClient } from "@tanstack/react-query";
import { MoreVertical, MoveRight } from "lucide-react";

import { ClusterTopology } from "./ClusterTopology";
import {
  dashboardGet,
  dashboardSave,
  type DashboardLayout,
  type DashboardPanel as Panel,
} from "../ipc/dashboards";
import {
  dashboardMetricDelete,
  dashboardMetricList,
  type DashboardMetricView,
} from "../ipc/dashboard_metrics";
import { LineChart, BarChart, GaugeChart, StatNumber } from "./widgets";
import type { OhlcPoint, SeriesPoint, KLineMode, KLineInterval } from "./widgets";
import { MetricConfigDialog } from "./MetricConfigDialog";

interface Props {
  applicationId: number;
}

const CELL_W = 80;
const CELL_H = 60;
const GRID_COLS = 12;
const GRID_ROWS = 8;

const PANEL_KINDS: Panel["kind"][] = [
  "kline",
  "active_jobs",
  "tasks_per_sec",
  "cpu",
  "pipeline_status",
  "audit_feed",
  "cluster_topology",
];

function defaultLayout(): DashboardLayout {
  const panels: Panel[] = [
    {
      id: "kline",
      kind: "kline",
      x: 0,
      y: 0,
      w: 6,
      h: 4,
      title: "K-line (BTC/USDT)",
    },
    {
      id: "active_jobs",
      kind: "active_jobs",
      x: 6,
      y: 0,
      w: 3,
      h: 2,
      title: "Active Jobs",
    },
    {
      id: "tasks_per_sec",
      kind: "tasks_per_sec",
      x: 9,
      y: 0,
      w: 3,
      h: 2,
      title: "Tasks/sec",
    },
    {
      id: "cpu",
      kind: "cpu",
      x: 6,
      y: 2,
      w: 3,
      h: 2,
      title: "CPU",
    },
    {
      id: "pipeline_status",
      kind: "pipeline_status",
      x: 0,
      y: 4,
      w: 6,
      h: 2,
      title: "Pipeline Status",
    },
  ];
  return { panels };
}

function newPanelId(): string {
  return `p-${Math.random().toString(36).slice(2, 9)}`;
}

export function DashboardEditor({ applicationId }: Props) {
  const qc = useQueryClient();
  const layoutQ = useQuery({
    queryKey: ["dashboard", applicationId],
    queryFn: () => dashboardGet(applicationId),
  });

  const [layout, setLayout] = useState<DashboardLayout | null>(null);
  const [savedOnce, setSavedOnce] = useState(false);
  const [editing, setEditing] = useState(true);
  const [menuFor, setMenuFor] = useState<string | null>(null);
  const [metricFor, setMetricFor] = useState<string | null>(null);
  const layoutRef = useRef<DashboardLayout | null>(null);

  useEffect(() => {
    if (layoutQ.data === undefined) return;
    if (layoutQ.data === null) {
      if (!savedOnce) {
        const initial = defaultLayout();
        layoutRef.current = initial;
        setLayout(initial);
        void dashboardSave(applicationId, JSON.stringify(initial)).then(() => {
          setSavedOnce(true);
          void qc.invalidateQueries({ queryKey: ["dashboard", applicationId] });
        });
      }
      return;
    }
    try {
      const parsed = JSON.parse(layoutQ.data.layout_json) as DashboardLayout;
      if (parsed && Array.isArray(parsed.panels)) {
        layoutRef.current = parsed;
        setLayout(parsed);
      } else {
        layoutRef.current = defaultLayout();
        setLayout(layoutRef.current);
      }
    } catch {
      layoutRef.current = defaultLayout();
      setLayout(layoutRef.current);
    }
  }, [layoutQ.data, savedOnce, applicationId, qc]);

  const saveLayout = async (next: DashboardLayout) => {
    setLayout(next);
    await dashboardSave(applicationId, JSON.stringify(next));
    setSavedOnce(true);
    await qc.invalidateQueries({ queryKey: ["dashboard", applicationId] });
  };

  const addPanel = (kind: Panel["kind"]) => {
    if (!layout) return;
    const id = newPanelId();
    const panel: Panel = {
      id,
      kind,
      x: 0,
      y: 0,
      w: 3,
      h: 2,
      title: titleFor(kind),
    };
    void saveLayout({ panels: [...layout.panels, panel] });
  };

  const removePanel = (id: string) => {
    if (!layout) return;
    void saveLayout({ panels: layout.panels.filter((p) => p.id !== id) });
    void dashboardMetricDelete(applicationId, id).catch(() => {});
  };

  const updatePanel = (id: string, patch: Partial<Panel>) => {
    const current = layoutRef.current;
    if (!current) return;
    const next = current.panels.map((p) =>
      p.id === id ? { ...p, ...patch } : p,
    );
    const nextLayout = { panels: next };
    layoutRef.current = nextLayout;
    setLayout(nextLayout);
  };

  const commitPanel = (_id: string) => {
    const current = layoutRef.current;
    if (!current) return;
    void saveLayout(current);
  };

  if (layout === null) {
    return (
      <div className="text-xs text-gray-400" data-testid="dashboard-loading">
        loading dashboard…
      </div>
    );
  }

  const widthPx = GRID_COLS * CELL_W;
  const heightPx = GRID_ROWS * CELL_H;

  return (
    <div className="space-y-3" data-testid="dashboard-editor">
      <header className="flex items-center justify-between">
        <h1 className="text-xl font-semibold">Dashboard</h1>
        <div className="flex items-center gap-2">
          {editing && (
            <select
              aria-label="Add panel"
              data-testid="add-panel-select"
              defaultValue=""
              onChange={(e) => {
                const v = e.target.value;
                if (v) {
                  addPanel(v as Panel["kind"]);
                  e.target.value = "";
                }
              }}
              className="px-2 py-1 text-xs rounded border border-gray-200 dark:border-neutral-700 bg-white dark:bg-neutral-900"
            >
              <option value="" disabled>
                Add panel…
              </option>
              {PANEL_KINDS.map((k) => (
                <option key={k} value={k}>
                  {titleFor(k)}
                </option>
              ))}
            </select>
          )}
          <button
            type="button"
            onClick={() => setEditing((e) => !e)}
            className="px-3 py-1 text-xs rounded border border-gray-200 dark:border-neutral-700"
            data-testid="toggle-edit"
          >
            {editing ? "View mode" : "Edit mode"}
          </button>
        </div>
      </header>

      <div
        className="relative rounded-md border border-gray-200 dark:border-neutral-700 bg-gray-50 dark:bg-neutral-900"
        style={{ width: widthPx, height: heightPx }}
        data-testid="dashboard-canvas"
      >
        {layout.panels.map((p) => (
          <PanelFrame
            key={p.id}
            panel={p}
            editing={editing}
            applicationId={applicationId}
            onMove={(dx, dy) => {
              const nx = clamp(p.x + dx, 0, GRID_COLS - p.w);
              const ny = clamp(p.y + dy, 0, GRID_ROWS - p.h);
              updatePanel(p.id, { x: nx, y: ny });
            }}
            onResize={(dw, dh) => {
              const nw = clamp(p.w + dw, 2, GRID_COLS - p.x);
              const nh = clamp(p.h + dh, 2, GRID_ROWS - p.y);
              updatePanel(p.id, { w: nw, h: nh });
            }}
            onCommit={() => commitPanel(p.id)}
            onRemove={() => removePanel(p.id)}
            onBindMetric={() => setMetricFor(p.id)}
            menuOpen={menuFor === p.id}
            onMenuToggle={() => setMenuFor(menuFor === p.id ? null : p.id)}
          />
        ))}
      </div>

      {metricFor !== null && layout !== null && (
        <MetricConfigDialog
          applicationId={applicationId}
          panelId={metricFor}
          onClose={() => {
            setMetricFor(null);
            void qc.invalidateQueries({
              queryKey: ["dashboard-metrics", applicationId],
            });
          }}
        />
      )}
    </div>
  );
}

function clamp(n: number, lo: number, hi: number): number {
  return Math.max(lo, Math.min(hi, n));
}

function titleFor(kind: Panel["kind"]): string {
  switch (kind) {
    case "kline":
      return "K-line";
    case "active_jobs":
      return "Active Jobs";
    case "tasks_per_sec":
      return "Tasks/sec";
    case "cpu":
      return "CPU";
    case "pipeline_status":
      return "Pipeline Status";
    case "audit_feed":
      return "Audit Feed";
    case "cluster_topology":
      return "Cluster Topology";
  }
}

interface FrameProps {
  panel: Panel;
  editing: boolean;
  applicationId: number;
  onMove(dx: number, dy: number): void;
  onResize(dw: number, dh: number): void;
  onCommit(): void;
  onRemove(): void;
  onBindMetric(): void;
  menuOpen: boolean;
  onMenuToggle(): void;
}

function PanelFrame({
  panel,
  editing,
  applicationId,
  onMove,
  onResize,
  onCommit,
  onRemove,
  onBindMetric,
  menuOpen,
  onMenuToggle,
}: FrameProps) {
  const ref = useRef<HTMLDivElement | null>(null);

  const onMouseDown = (e: React.MouseEvent, mode: "move" | "resize") => {
    if (!editing) return;
    e.preventDefault();
    e.stopPropagation();
    const startX = e.clientX;
    const startY = e.clientY;
    let lastDx = 0;
    let lastDy = 0;
    const onMove2 = (ev: MouseEvent) => {
      const dx = Math.round((ev.clientX - startX) / CELL_W);
      const dy = Math.round((ev.clientY - startY) / CELL_H);
      if (dx === lastDx && dy === lastDy) return;
      const incDx = dx - lastDx;
      const incDy = dy - lastDy;
      lastDx = dx;
      lastDy = dy;
      if (mode === "move") onMove(incDx, incDy);
      else onResize(incDx, incDy);
    };
    const onUp = () => {
      document.removeEventListener("mousemove", onMove2);
      document.removeEventListener("mouseup", onUp);
      onCommit();
    };
    document.addEventListener("mousemove", onMove2);
    document.addEventListener("mouseup", onUp);
  };

  return (
    <div
      ref={ref}
      data-testid={`panel-${panel.id}`}
      data-kind={panel.kind}
      className={[
        "absolute rounded-md border bg-white dark:bg-neutral-800",
        editing
          ? "border-dashed border-accent-blue/60"
          : "border-gray-200 dark:border-neutral-700",
      ].join(" ")}
      style={{
        left: panel.x * CELL_W,
        top: panel.y * CELL_H,
        width: panel.w * CELL_W,
        height: panel.h * CELL_H,
      }}
    >
      <header
        className="flex items-center justify-between px-2 py-1 text-[10px] font-semibold text-gray-500 dark:text-neutral-400 border-b border-gray-100 dark:border-neutral-700"
        onMouseDown={(e) => onMouseDown(e, "move")}
      >
        <span className="truncate">{panel.title}</span>
        {editing && (
          <>
            <button
              type="button"
              aria-label="Panel menu"
              data-testid={`panel-menu-${panel.id}`}
              onClick={(e) => {
                e.stopPropagation();
                onMenuToggle();
              }}
              className="p-0.5 rounded hover:bg-gray-100 dark:hover:bg-neutral-700"
            >
              <MoreVertical size={10} />
            </button>
            {menuOpen && (
              <div
                className="absolute right-1 top-5 z-10 rounded-md border border-gray-200 dark:border-neutral-700 bg-white dark:bg-neutral-800 shadow-lg text-xs"
                onMouseDown={(e) => e.stopPropagation()}
              >
                <button
                  type="button"
                  data-testid={`panel-bind-${panel.id}`}
                  onClick={(e) => {
                    e.stopPropagation();
                    onBindMetric();
                    onMenuToggle();
                  }}
                  className="block w-full text-left px-3 py-1.5 hover:bg-gray-100 dark:hover:bg-neutral-700"
                >
                  Bind metric
                </button>
                <button
                  type="button"
                  data-testid={`panel-remove-${panel.id}`}
                  onClick={(e) => {
                    e.stopPropagation();
                    onRemove();
                  }}
                  className="block w-full text-left px-3 py-1.5 hover:bg-gray-100 dark:hover:bg-neutral-700"
                >
                  Remove panel
                </button>
              </div>
            )}
          </>
        )}
      </header>
      <div className="p-1 h-[calc(100%-1.4rem)]">
        <PanelBody panel={panel} applicationId={applicationId} />
      </div>
      {editing && (
        <button
          type="button"
          aria-label="Resize"
          data-testid={`panel-resize-${panel.id}`}
          onMouseDown={(e) => onMouseDown(e, "resize")}
          className="absolute right-0 bottom-0 p-1 text-gray-400 hover:text-accent-blue cursor-se-resize"
        >
          <MoveRight size={10} />
        </button>
      )}
    </div>
  );
}

function PanelBody({
  panel,
  applicationId,
}: {
  panel: Panel;
  applicationId: number;
}) {
  return (
    <MetricPanelContent
      applicationId={applicationId}
      panelId={panel.id}
      kind={panel.kind}
    />
  );
}

function MetricPanelContent({
  applicationId,
  panelId,
  kind,
}: {
  applicationId: number;
  panelId: string;
  kind: Panel["kind"];
}) {
  const metricsQ = useQuery<DashboardMetricView[]>({
    queryKey: ["dashboard-metrics", applicationId],
    queryFn: () => dashboardMetricList(applicationId),
  });
  const metric = metricsQ.data?.find((m) => m.panel_id === panelId);
  if (metric) {
    return <MetricWidget metric={metric} />;
  }
  return <StaticWidget kind={kind} />;
}

function MetricWidget({ metric }: { metric: DashboardMetricView }) {
  const cfg = parseConfig(metric.chart_config_json);
  if (metric.widget_kind === "line_chart") {
    return (
      <LineChart
        points={sampleKlinePoints()}
        mode={cfg.mode ?? "line"}
        interval={cfg.interval ?? "1m"}
      />
    );
  }
  if (metric.widget_kind === "kline") {
    return (
      <LineChart
        points={sampleKlineOhlc()}
        mode={cfg.mode ?? "candlestick"}
        interval={cfg.interval ?? "1m"}
      />
    );
  }
  if (metric.widget_kind === "bar_chart") {
    return <BarChart data={sampleBar()} title={cfg.title} color={cfg.color} />;
  }
  if (metric.widget_kind === "gauge") {
    return (
      <GaugeChart
        value={cfg.value ?? 50}
        unit={cfg.unit ?? "%"}
        title={cfg.title}
        color={cfg.color}
      />
    );
  }
  return <StatNumber value={cfg.value ?? 0} label={cfg.title ?? metric.source_field} />;
}

function StaticWidget({ kind }: { kind: Panel["kind"] }) {
  switch (kind) {
    case "kline":
      return <LineChart points={sampleKlineOhlc()} mode="candlestick" />;
    case "active_jobs":
      return <StatNumber value={42} label="Active Jobs" />;
    case "tasks_per_sec":
      return <GaugeChart value={1.4} min={0} max={5} unit="/s" title="Tasks/sec" />;
    case "cpu":
      return <GaugeChart value={23} unit="%" title="CPU" />;
    case "pipeline_status":
      return <StatNumber value={3} label="Running / Failed" unit="/0" />;
    case "audit_feed":
      return (
        <p className="text-[10px] text-gray-400 px-2 py-4 text-center">
          Audit feed placeholder
        </p>
      );
    case "cluster_topology":
      return (
        <div className="h-full -m-1">
          <ClusterTopology
            nodes={[
              { id: 1, role: "Leader", addr: "127.0.0.1:9001", term: 4, lag: 0 },
              { id: 2, role: "Follower", addr: "127.0.0.1:9002", term: 4, lag: 1 },
              { id: 3, role: "Follower", addr: "127.0.0.1:9003", term: 4, lag: 2 },
            ]}
            leaderId={1}
          />
        </div>
      );
  }
}

function parseConfig(json: string): {
  title?: string;
  color?: string;
  value?: number;
  unit?: string;
  mode?: KLineMode;
  interval?: KLineInterval;
} {
  try {
    return JSON.parse(json) as {
      title?: string;
      color?: string;
      value?: number;
      unit?: string;
      mode?: KLineMode;
      interval?: KLineInterval;
    };
  } catch {
    return {};
  }
}

function sampleKlinePoints(): SeriesPoint[] {
  const out: SeriesPoint[] = [];
  const now = Date.now();
  for (let i = 0; i < 40; i += 1) {
    out.push({
      ts: now - (40 - i) * 60_000,
      value: 100 + Math.sin(i / 3) * 5 + i * 0.2,
    });
  }
  return out;
}

function sampleKlineOhlc(): OhlcPoint[] {
  const out: OhlcPoint[] = [];
  const now = Date.now();
  let prev = 100;
  for (let i = 0; i < 40; i += 1) {
    const drift = Math.sin(i / 3) * 5 + i * 0.2;
    const open = prev;
    const close = prev + drift;
    const high = Math.max(open, close) + Math.abs(Math.cos(i / 2)) * 3;
    const low = Math.min(open, close) - Math.abs(Math.sin(i / 2)) * 2;
    out.push({
      ts: now - (40 - i) * 60_000,
      open,
      high,
      low,
      close,
      volume: 1000 + i * 10,
    });
    prev = close;
  }
  return out;
}

function sampleBar(): { label: string; value: number }[] {
  return [
    { label: "queued", value: 1 },
    { label: "running", value: 3 },
    { label: "historical", value: 12 },
    { label: "failed", value: 0 },
  ];
}