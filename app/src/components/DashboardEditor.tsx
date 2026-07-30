import { useEffect, useRef, useState } from "react";
import { useQuery, useQueryClient } from "@tanstack/react-query";
import { MoreVertical, GripHorizontal, MoveRight } from "lucide-react";

import { ClusterTopology } from "./ClusterTopology";
import {
  dashboardGet,
  dashboardSave,
  type DashboardLayout,
  type DashboardPanel as Panel,
} from "../ipc/dashboards";

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
            menuOpen={menuFor === p.id}
            onMenuToggle={() => setMenuFor(menuFor === p.id ? null : p.id)}
          />
        ))}
      </div>
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
  onMove(dx: number, dy: number): void;
  onResize(dw: number, dh: number): void;
  onCommit(): void;
  onRemove(): void;
  menuOpen: boolean;
  onMenuToggle(): void;
}

function PanelFrame({
  panel,
  editing,
  onMove,
  onResize,
  onCommit,
  onRemove,
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
      <div className="p-1 text-[10px] text-gray-400 h-[calc(100%-1.4rem)]">
        <PanelBody panel={panel} />
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

function PanelBody({ panel }: { panel: Panel }) {
  switch (panel.kind) {
    case "kline":
      return (
        <div className="h-full">
          <GripHorizontal size={40} className="mx-auto mt-2 text-gray-300" />
          <p className="text-center text-[10px]">kline chart placeholder</p>
        </div>
      );
    case "active_jobs":
      return <p>42 jobs</p>;
    case "tasks_per_sec":
      return <p>1.4 K/sec</p>;
    case "cpu":
      return <p>23%</p>;
    case "pipeline_status":
      return <p>3 running / 0 failed</p>;
    case "audit_feed":
      return <p>audit feed placeholder</p>;
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
