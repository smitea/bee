import { useEffect, useState } from "react";
import { useQuery } from "@tanstack/react-query";
import { X, Save } from "lucide-react";

import {
  dashboardMetricGet,
  dashboardMetricSave,
  type DashboardMetricView,
} from "../ipc/dashboard_metrics";
import { listJobs } from "../ipc/cluster";
import { useConnection } from "../state/connectionStore";
import type { KLineMode, KLineInterval } from "./widgets";

interface Props {
  applicationId: number;
  panelId: string;
  onClose(): void;
}

const WIDGET_KINDS = [
  { id: "line_chart", label: "Line Chart" },
  { id: "kline", label: "K-Line" },
  { id: "bar_chart", label: "Bar Chart" },
  { id: "gauge", label: "Gauge" },
  { id: "stat", label: "Stat Number" },
] as const;

type WidgetKind = (typeof WIDGET_KINDS)[number]["id"];

const KLINE_MODES: { id: KLineMode; label: string }[] = [
  { id: "candlestick", label: "Candlestick" },
  { id: "line", label: "Line" },
];

const KLINE_INTERVALS: { id: KLineInterval; label: string }[] = [
  { id: "1m", label: "1 minute" },
  { id: "5m", label: "5 minutes" },
  { id: "1h", label: "1 hour" },
  { id: "1d", label: "1 day" },
];

export function MetricConfigDialog({ applicationId, panelId, onClose }: Props) {
  const addr = useConnection((s) => s.addr);
  const jobsQ = useQuery({
    queryKey: ["jobs", addr],
    queryFn: () => listJobs(addr),
  });

  const [existing, setExisting] = useState<DashboardMetricView | null>(null);
  const [jobId, setJobId] = useState<string>("");
  const [sourceField, setSourceField] = useState<string>("price");
  const [widgetKind, setWidgetKind] = useState<WidgetKind>("line_chart");
  const [title, setTitle] = useState<string>("");
  const [color, setColor] = useState<string>("#3b82f6");
  const [unit, setUnit] = useState<string>("");
  const [klineMode, setKlineMode] = useState<KLineMode>("candlestick");
  const [klineInterval, setKlineInterval] = useState<KLineInterval>("1m");
  const [error, setError] = useState<string | null>(null);
  const [busy, setBusy] = useState(false);

  useEffect(() => {
    let cancelled = false;
    void dashboardMetricGet(applicationId, panelId).then((m) => {
      if (cancelled || !m) return;
      setExisting(m);
      setJobId(m.pipeline_job_id ? String(m.pipeline_job_id) : "");
      setSourceField(m.source_field);
      setWidgetKind(m.widget_kind as WidgetKind);
      try {
        const cfg = JSON.parse(m.chart_config_json);
        if (cfg.title) setTitle(String(cfg.title));
        if (cfg.color) setColor(String(cfg.color));
        if (cfg.unit) setUnit(String(cfg.unit));
        if (cfg.mode === "candlestick" || cfg.mode === "line") {
          setKlineMode(cfg.mode);
        }
        if (
          cfg.interval === "1m" ||
          cfg.interval === "5m" ||
          cfg.interval === "1h" ||
          cfg.interval === "1d"
        ) {
          setKlineInterval(cfg.interval);
        }
      } catch {}
    });
    return () => {
      cancelled = true;
    };
  }, [applicationId, panelId]);

  const onSave = async () => {
    setBusy(true);
    setError(null);
    try {
      await dashboardMetricSave(
        applicationId,
        panelId,
        jobId ? Number(jobId) : null,
        sourceField,
        widgetKind,
        JSON.stringify({
          title,
          color,
          unit,
          mode: klineMode,
          interval: klineInterval,
        }),
      );
      setBusy(false);
      onClose();
    } catch (e) {
      setError(String(e));
      setBusy(false);
    }
  };

  return (
    <div
      className="fixed inset-0 z-50 flex items-center justify-center bg-black/40"
      role="dialog"
      aria-modal="true"
      aria-label="Metric configuration"
      data-testid="metric-config-dialog"
    >
      <div className="bg-white dark:bg-neutral-800 rounded-lg shadow-xl w-[520px] max-w-[95vw] max-h-[90vh] flex flex-col">
        <header className="flex items-center justify-between px-4 py-3 border-b border-gray-200 dark:border-neutral-700">
          <h2 className="text-sm font-semibold">Bind metric · {panelId}</h2>
          <button
            type="button"
            onClick={onClose}
            aria-label="Close"
            className="p-1 rounded text-gray-500 hover:bg-gray-100 dark:hover:bg-neutral-700"
          >
            <X size={14} />
          </button>
        </header>

        <div className="p-4 space-y-3 text-xs overflow-y-auto">
          <Field label="Pipeline Job">
            <select
              aria-label="pipeline job"
              value={jobId}
              onChange={(e) => setJobId(e.target.value)}
              className="flex-1 px-2 py-1 rounded border border-gray-200 dark:border-neutral-700 bg-white dark:bg-neutral-900 font-mono"
            >
              <option value="">— none —</option>
              {(jobsQ.data ?? []).map((j) => (
                <option key={j.job_id} value={j.job_id}>
                  #{j.job_id} · {j.dag_hash.slice(0, 8)}
                </option>
              ))}
            </select>
          </Field>

          <Field label="Source Field">
            <input
              aria-label="source field"
              value={sourceField}
              onChange={(e) => setSourceField(e.target.value)}
              className="flex-1 px-2 py-1 rounded border border-gray-200 dark:border-neutral-700 bg-white dark:bg-neutral-900 font-mono"
            />
          </Field>

          <Field label="Widget Kind">
            <select
              aria-label="widget kind"
              value={widgetKind}
              onChange={(e) => setWidgetKind(e.target.value as WidgetKind)}
              className="flex-1 px-2 py-1 rounded border border-gray-200 dark:border-neutral-700 bg-white dark:bg-neutral-900"
            >
              {WIDGET_KINDS.map((w) => (
                <option key={w.id} value={w.id}>
                  {w.label}
                </option>
              ))}
            </select>
          </Field>

          <Field label="Title (optional)">
            <input
              aria-label="title"
              value={title}
              onChange={(e) => setTitle(e.target.value)}
              className="flex-1 px-2 py-1 rounded border border-gray-200 dark:border-neutral-700 bg-white dark:bg-neutral-900"
            />
          </Field>

          <Field label="Color">
            <input
              aria-label="color"
              type="color"
              value={color}
              onChange={(e) => setColor(e.target.value)}
              className="w-12 h-6 rounded border border-gray-200 dark:border-neutral-700 bg-white dark:bg-neutral-900"
            />
          </Field>

          <Field label="Unit (optional)">
            <input
              aria-label="unit"
              value={unit}
              onChange={(e) => setUnit(e.target.value)}
              placeholder="%, /s, ..."
              className="flex-1 px-2 py-1 rounded border border-gray-200 dark:border-neutral-700 bg-white dark:bg-neutral-900 font-mono"
            />
          </Field>

          {widgetKind === "kline" && (
            <Field label="K-Line Mode">
              <select
                aria-label="kline mode"
                value={klineMode}
                onChange={(e) => setKlineMode(e.target.value as KLineMode)}
                className="flex-1 px-2 py-1 rounded border border-gray-200 dark:border-neutral-700 bg-white dark:bg-neutral-900"
              >
                {KLINE_MODES.map((m) => (
                  <option key={m.id} value={m.id}>
                    {m.label}
                  </option>
                ))}
              </select>
            </Field>
          )}

          {widgetKind === "kline" && (
            <Field label="Time Interval">
              <select
                aria-label="kline interval"
                value={klineInterval}
                onChange={(e) => setKlineInterval(e.target.value as KLineInterval)}
                className="flex-1 px-2 py-1 rounded border border-gray-200 dark:border-neutral-700 bg-white dark:bg-neutral-900"
              >
                {KLINE_INTERVALS.map((i) => (
                  <option key={i.id} value={i.id}>
                    {i.label}
                  </option>
                ))}
              </select>
            </Field>
          )}

          {error && <div className="text-accent-red">{error}</div>}
        </div>

        <footer className="px-4 py-3 border-t border-gray-200 dark:border-neutral-700 flex items-center justify-end gap-2">
          <button
            type="button"
            onClick={onClose}
            className="px-3 py-1 text-xs rounded border border-gray-200 dark:border-neutral-700"
          >
            Cancel
          </button>
          <button
            type="button"
            onClick={() => void onSave()}
            disabled={busy}
            className="flex items-center gap-1 px-3 py-1 text-xs rounded bg-accent-blue text-white hover:bg-accent-blue/90 disabled:opacity-50"
          >
            <Save size={11} />
            Save
          </button>
        </footer>
        {existing && (
          <div className="text-[10px] text-gray-400 px-4 pb-3">
            Last saved: {existing.source_field} → {existing.widget_kind}
          </div>
        )}
      </div>
    </div>
  );
}

function Field({
  label,
  children,
}: {
  label: string;
  children: React.ReactNode;
}) {
  return (
    <div className="flex items-center gap-3 py-1">
      <label className="w-40 text-xs text-gray-600 dark:text-neutral-400">
        {label}
      </label>
      {children}
    </div>
  );
}