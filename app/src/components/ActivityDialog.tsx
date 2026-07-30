import { useEffect, useMemo, useState } from "react";
import { X, RefreshCw } from "lucide-react";

import { useAudit, navAction, summary } from "../state/auditStore";
import { useTabs, type TabKind } from "../state/tabsStore";
import type { AuditEventView } from "../ipc/audit";

export interface ActivityDialogProps {
  onClose: () => void;
  navigate?: (kind: string, resourceId: string | null) => void;
}

type ResultFilter = "all" | "success" | "errors";

interface FilterState {
  result: ResultFilter;
  category: string;
  dateFrom: string;
  dateTo: string;
}

const PAGE_SIZE = 20;

export function ActivityDialog({ onClose, navigate }: ActivityDialogProps) {
  const events = useAudit((s) => s.events);
  const refresh = useAudit((s) => s.refresh);
  const openTab = useTabs((s) => s.open);

  const [filter, setFilter] = useState<FilterState>({
    result: "all",
    category: "all",
    dateFrom: "",
    dateTo: "",
  });
  const [pageSize, setPageSize] = useState(PAGE_SIZE);

  useEffect(() => {
    const onKey = (e: KeyboardEvent) => {
      if (e.key === "Escape") onClose();
    };
    window.addEventListener("keydown", onKey);
    return () => window.removeEventListener("keydown", onKey);
  }, [onClose]);

  const categories = useMemo(() => {
    const set = new Set<string>();
    for (const e of events) {
      const idx = e.action.indexOf(".");
      if (idx > 0) set.add(e.action.slice(0, idx));
    }
    return Array.from(set).sort();
  }, [events]);

  const filtered = useMemo(() => {
    return events.filter((e) => {
      if (filter.result === "success" && e.result !== "Success") return false;
      if (filter.result === "errors" && e.result === "Success") return false;
      if (filter.category !== "all") {
        const idx = e.action.indexOf(".");
        const prefix = idx >= 0 ? e.action.slice(0, idx) : e.action;
        if (prefix !== filter.category) return false;
      }
      if (filter.dateFrom) {
        const from = parseDateStart(filter.dateFrom);
        if (e.timestamp < from) return false;
      }
      if (filter.dateTo) {
        const to = parseDateEnd(filter.dateTo);
        if (e.timestamp > to) return false;
      }
      return true;
    });
  }, [events, filter]);

  const visible = filtered.slice(0, pageSize);
  const hasMore = filtered.length > pageSize;

  const onNavigate = (ev: AuditEventView) => {
    const target = navAction(ev);
    if (!target) return;
    if (navigate) {
      navigate(target.kind, target.resourceId);
    } else {
      void openTab({
        kind: target.kind as TabKind,
        resourceId: target.resourceId,
        title: summary(ev),
      });
    }
    onClose();
  };

  return (
    <div
      className="fixed inset-0 z-50 flex items-center justify-center bg-black/40"
      role="dialog"
      aria-modal="true"
      aria-label="Activity"
      onClick={onClose}
    >
      <div
        className="bg-white dark:bg-neutral-800 rounded-lg shadow-xl w-[820px] max-w-[95vw] max-h-[85vh] flex flex-col"
        onClick={(e) => e.stopPropagation()}
      >
        <header className="flex items-center justify-between px-4 py-3 border-b border-gray-200 dark:border-neutral-700">
          <h2 className="text-sm font-semibold">Activity</h2>
          <div className="flex items-center gap-2">
            <button
              onClick={() => void refresh(200)}
              className="p-1 rounded text-gray-500 hover:bg-gray-100 dark:hover:bg-neutral-700"
              title="Refresh"
              aria-label="Refresh activity"
            >
              <RefreshCw size={12} />
            </button>
            <button
              onClick={onClose}
              className="p-1 rounded text-gray-500 hover:bg-gray-100 dark:hover:bg-neutral-700"
              aria-label="Close"
            >
              <X size={14} />
            </button>
          </div>
        </header>

        <div className="flex items-center gap-2 px-4 py-2 border-b border-gray-200 dark:border-neutral-700 text-xs">
          <select
            value={filter.result}
            onChange={(e) =>
              setFilter((f) => ({ ...f, result: e.target.value as ResultFilter }))
            }
            className="px-2 py-1 rounded border border-gray-200 dark:border-neutral-700 bg-white dark:bg-neutral-900"
            aria-label="Filter by result"
          >
            <option value="all">All results</option>
            <option value="success">Success</option>
            <option value="errors">Errors only</option>
          </select>
          <select
            value={filter.category}
            onChange={(e) =>
              setFilter((f) => ({ ...f, category: e.target.value }))
            }
            className="px-2 py-1 rounded border border-gray-200 dark:border-neutral-700 bg-white dark:bg-neutral-900"
            aria-label="Filter by action category"
          >
            <option value="all">All categories</option>
            {categories.map((c) => (
              <option key={c} value={c}>
                {c}
              </option>
            ))}
          </select>
          <input
            type="date"
            value={filter.dateFrom}
            onChange={(e) =>
              setFilter((f) => ({ ...f, dateFrom: e.target.value }))
            }
            className="px-2 py-1 rounded border border-gray-200 dark:border-neutral-700 bg-white dark:bg-neutral-900"
            aria-label="Date from"
          />
          <span className="text-gray-400">–</span>
          <input
            type="date"
            value={filter.dateTo}
            onChange={(e) =>
              setFilter((f) => ({ ...f, dateTo: e.target.value }))
            }
            className="px-2 py-1 rounded border border-gray-200 dark:border-neutral-700 bg-white dark:bg-neutral-900"
            aria-label="Date to"
          />
        </div>

        <div className="flex-1 overflow-y-auto px-2 py-2">
          {visible.length === 0 ? (
            <p className="text-xs text-gray-400 px-2 py-6 text-center">
              {events.length === 0 ? "No activity yet" : "No matching events"}
            </p>
          ) : (
            <ul className="divide-y divide-gray-100 dark:divide-neutral-700">
              {visible.map((e) => (
                <Row key={e.id} ev={e} onNavigate={() => onNavigate(e)} />
              ))}
            </ul>
          )}
        </div>

        {hasMore && (
          <div className="px-4 py-2 border-t border-gray-200 dark:border-neutral-700 text-center">
            <button
              type="button"
              onClick={() => setPageSize((n) => n + PAGE_SIZE)}
              className="px-3 py-1 text-xs rounded border border-gray-200 dark:border-neutral-700 hover:bg-gray-50 dark:hover:bg-neutral-700"
            >
              Load more ({filtered.length - pageSize} remaining)
            </button>
          </div>
        )}
      </div>
    </div>
  );
}

function Row({ ev, onNavigate }: { ev: AuditEventView; onNavigate(): void }) {
  const target = navAction(ev);
  return (
    <li className="px-2 py-2 text-xs hover:bg-gray-50 dark:hover:bg-neutral-700/50">
      <div className="flex items-center gap-2">
        <span
          className={[
            "inline-block w-1.5 h-1.5 rounded-full shrink-0",
            ev.result === "Success" ? "bg-accent-green" : "bg-accent-red",
          ].join(" ")}
        />
        <span className="font-medium truncate flex-1">{summary(ev)}</span>
        <span className="text-[10px] text-gray-400">{formatTime(ev.timestamp)}</span>
      </div>
      <div className="mt-1 pl-3.5 flex items-center gap-2 text-[10px] text-gray-500 dark:text-neutral-400 flex-wrap">
        <span className="font-mono">{ev.action}</span>
        <span>·</span>
        <span>{ev.actor}</span>
        <span>·</span>
        <span>{formatDateTime(ev.timestamp)}</span>
        <span
          className={[
            "px-1.5 py-0.5 rounded text-[9px] uppercase tracking-wider",
            ev.result === "Success"
              ? "bg-accent-green/15 text-accent-green"
              : "bg-accent-red/15 text-accent-red",
          ].join(" ")}
        >
          {ev.result}
        </span>
      </div>
      <div className="mt-1 pl-3.5 grid grid-cols-[7rem_1fr] gap-x-2 gap-y-0.5 text-[10px]">
        {detailRows(ev).map(([k, v]) => (
          <FragmentRow key={k} k={k} v={v} />
        ))}
      </div>
      {target && (
        <div className="mt-1 pl-3.5 flex justify-end">
          <button
            type="button"
            onClick={onNavigate}
            className="text-accent-blue hover:underline"
            title="Go to related page"
          >
            {target.label}
          </button>
        </div>
      )}
    </li>
  );
}

function FragmentRow({ k, v }: { k: string; v: string }) {
  return (
    <>
      <span className="text-gray-500 dark:text-neutral-400">{k}</span>
      <span className="font-mono text-gray-700 dark:text-neutral-200 break-all">
        {v}
      </span>
    </>
  );
}

function detailRows(ev: AuditEventView): [string, string][] {
  const rows: [string, string][] = [
    ["resource_kind", ev.resource_kind ?? "—"],
    ["resource_id", ev.resource_id ?? "—"],
    ["application_id", ev.application_id === null ? "—" : String(ev.application_id)],
    ["correlation_id", ev.correlation_id ?? "—"],
    ["operation_id", ev.operation_id ?? "—"],
  ];
  return rows;
}

function formatTime(ts: number): string {
  const d = new Date(ts * 1000);
  const pad = (n: number) => String(n).padStart(2, "0");
  return `${pad(d.getHours())}:${pad(d.getMinutes())}:${pad(d.getSeconds())}`;
}

function formatDateTime(ts: number): string {
  const d = new Date(ts * 1000);
  const pad = (n: number) => String(n).padStart(2, "0");
  return `${d.getFullYear()}-${pad(d.getMonth() + 1)}-${pad(d.getDate())} ${formatTime(ts)}`;
}

function parseDateStart(iso: string): number {
  const d = new Date(`${iso}T00:00:00`);
  return Math.floor(d.getTime() / 1000);
}

function parseDateEnd(iso: string): number {
  const d = new Date(`${iso}T23:59:59`);
  return Math.floor(d.getTime() / 1000);
}
