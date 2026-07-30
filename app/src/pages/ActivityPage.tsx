import { useMemo, useState } from "react";
import { useQuery } from "@tanstack/react-query";
import {
  Activity,
  ArrowDown,
  ArrowUp,
  ArrowUpDown,
  ExternalLink,
  Loader2,
  RefreshCw,
  Search,
  X,
} from "lucide-react";

import { useConnection } from "../state/connectionStore";
import { useTabs, type TabKind } from "../state/tabsStore";
import { auditList, type AuditEventView } from "../ipc/audit";
import { categoryOf, navAction, summary } from "../state/auditStore";

const PAGE_SIZE = 30;
const POLL_MS = 5000;

type ResultFilter = "all" | "success" | "failure";
type SortKey = "timestamp" | "action" | "actor" | "resource" | "result";
type SortDir = "asc" | "desc";

interface FilterState {
  result: ResultFilter;
  category: string;
  query: string;
  dateFrom: string;
  dateTo: string;
}

interface SortState {
  key: SortKey;
  dir: SortDir;
}

interface ColumnDef {
  key: SortKey;
  label: string;
  className?: string;
}

const COLUMNS: ColumnDef[] = [
  { key: "timestamp", label: "Time" },
  { key: "action", label: "Action" },
  { key: "actor", label: "Actor" },
  { key: "resource", label: "Resource" },
  { key: "result", label: "Result" },
];

export function ActivityPage() {
  const addr = useConnection((s) => s.addr);
  const openTab = useTabs((s) => s.open);

  const eventsQ = useQuery<AuditEventView[]>({
    queryKey: ["audit-events", addr],
    queryFn: () => auditList(500),
    refetchInterval: POLL_MS,
  });

  const events = eventsQ.data ?? [];

  const [filter, setFilter] = useState<FilterState>({
    result: "all",
    category: "all",
    query: "",
    dateFrom: "",
    dateTo: "",
  });
  const [sort, setSort] = useState<SortState>({ key: "timestamp", dir: "desc" });
  const [pageSize, setPageSize] = useState(PAGE_SIZE);
  const [selected, setSelected] = useState<AuditEventView | null>(null);

  const categories = useMemo(() => {
    const set = new Set<string>();
    for (const ev of events) set.add(categoryOf(ev.action));
    return Array.from(set).sort();
  }, [events]);

  const filtered = useMemo(() => {
    const q = filter.query.trim().toLowerCase();
    return events.filter((e) => {
      if (filter.result === "success" && e.result !== "Success") return false;
      if (filter.result === "failure" && e.result === "Success") return false;
      if (filter.category !== "all" && categoryOf(e.action) !== filter.category) {
        return false;
      }
      if (q.length > 0) {
        const hay =
          `${e.summary} ${e.action} ${e.actor} ${e.resource_kind ?? ""} ${e.resource_id ?? ""}`.toLowerCase();
        if (!hay.includes(q)) return false;
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

  const sorted = useMemo(() => {
    const arr = [...filtered];
    arr.sort((a, b) => compare(a, b, sort));
    return arr;
  }, [filtered, sort]);

  const visible = sorted.slice(0, pageSize);
  const hasMore = sorted.length > pageSize;

  const onToggleSort = (key: SortKey) => {
    setSort((s) =>
      s.key === key ? { key, dir: s.dir === "asc" ? "desc" : "asc" } : { key, dir: "asc" },
    );
  };

  const onOpenInTab = async (ev: AuditEventView) => {
    const target = navAction(ev);
    if (!target) return;
    const kind = target.kind as TabKind;
    await openTab({
      kind,
      resourceId: target.resourceId,
      title: summary(ev),
    });
  };

  const onRefresh = () => {
    void eventsQ.refetch();
  };

  return (
    <section
      className="flex flex-col h-full min-w-0 space-y-3"
      data-testid="activity-page"
    >
      <header className="flex items-center justify-between">
        <div className="flex items-center gap-2">
          <Activity size={16} className="text-accent-blue" />
          <h1 className="text-base font-semibold">Recent Activity</h1>
          <span className="text-[10px] text-gray-500 dark:text-neutral-400">
            {eventsQ.isFetching ? "syncing…" : `${filtered.length} event(s)`}
          </span>
        </div>
        <div className="flex items-center gap-1">
          <button
            type="button"
            onClick={onRefresh}
            disabled={eventsQ.isFetching}
            aria-label="Refresh activity"
            title="Refresh"
            className="p-1.5 rounded text-gray-500 hover:bg-gray-100 dark:hover:bg-neutral-700 disabled:opacity-50"
          >
            <RefreshCw
              size={12}
              className={eventsQ.isFetching ? "animate-spin" : ""}
            />
          </button>
        </div>
      </header>

      <div className="flex flex-wrap items-center gap-2 px-3 py-2 rounded border border-gray-200 dark:border-neutral-700 bg-white dark:bg-neutral-800 text-xs">
        <label className="flex items-center gap-1">
          <span className="text-[10px] uppercase tracking-wider text-gray-500 dark:text-neutral-400">
            Result
          </span>
          <select
            value={filter.result}
            onChange={(e) =>
              setFilter((f) => ({ ...f, result: e.target.value as ResultFilter }))
            }
            className="px-2 py-1 rounded border border-gray-200 dark:border-neutral-700 bg-white dark:bg-neutral-900"
            aria-label="Filter by result"
          >
            <option value="all">All</option>
            <option value="success">Success</option>
            <option value="failure">Failure</option>
          </select>
        </label>
        <label className="flex items-center gap-1">
          <span className="text-[10px] uppercase tracking-wider text-gray-500 dark:text-neutral-400">
            Category
          </span>
          <select
            value={filter.category}
            onChange={(e) => setFilter((f) => ({ ...f, category: e.target.value }))}
            className="px-2 py-1 rounded border border-gray-200 dark:border-neutral-700 bg-white dark:bg-neutral-900"
            aria-label="Filter by action category"
          >
            <option value="all">All</option>
            {categories.map((c) => (
              <option key={c} value={c}>
                {c}
              </option>
            ))}
          </select>
        </label>
        <label className="flex items-center gap-1 flex-1 min-w-[10rem]">
          <span className="text-[10px] uppercase tracking-wider text-gray-500 dark:text-neutral-400 sr-only">
            Search
          </span>
          <div className="relative flex-1">
            <Search
              size={11}
              className="absolute left-2 top-1/2 -translate-y-1/2 text-gray-400"
            />
            <input
              type="text"
              value={filter.query}
              onChange={(e) => setFilter((f) => ({ ...f, query: e.target.value }))}
              placeholder="Search summary, action, actor, resource…"
              className="w-full pl-7 pr-2 py-1 rounded border border-gray-200 dark:border-neutral-700 bg-white dark:bg-neutral-900"
              aria-label="Free-text search"
            />
          </div>
        </label>
        <label className="flex items-center gap-1">
          <span className="text-[10px] uppercase tracking-wider text-gray-500 dark:text-neutral-400">
            From
          </span>
          <input
            type="date"
            value={filter.dateFrom}
            onChange={(e) => setFilter((f) => ({ ...f, dateFrom: e.target.value }))}
            className="px-2 py-1 rounded border border-gray-200 dark:border-neutral-700 bg-white dark:bg-neutral-900"
            aria-label="Date from"
          />
        </label>
        <label className="flex items-center gap-1">
          <span className="text-[10px] uppercase tracking-wider text-gray-500 dark:text-neutral-400">
            To
          </span>
          <input
            type="date"
            value={filter.dateTo}
            onChange={(e) => setFilter((f) => ({ ...f, dateTo: e.target.value }))}
            className="px-2 py-1 rounded border border-gray-200 dark:border-neutral-700 bg-white dark:bg-neutral-900"
            aria-label="Date to"
          />
        </label>
        {(filter.result !== "all" ||
          filter.category !== "all" ||
          filter.query ||
          filter.dateFrom ||
          filter.dateTo) && (
          <button
            type="button"
            onClick={() =>
              setFilter({
                result: "all",
                category: "all",
                query: "",
                dateFrom: "",
                dateTo: "",
              })
            }
            className="text-[10px] text-accent-blue hover:underline"
          >
            Clear filters
          </button>
        )}
      </div>

      <div className="flex-1 min-h-0 overflow-auto rounded border border-gray-200 dark:border-neutral-700 bg-white dark:bg-neutral-800">
        <table className="w-full text-xs">
          <thead className="sticky top-0 bg-gray-50 dark:bg-neutral-900 text-[10px] uppercase tracking-wider text-gray-500 dark:text-neutral-400 border-b border-gray-200 dark:border-neutral-700">
            <tr>
              {COLUMNS.map((col) => {
                const active = sort.key === col.key;
                return (
                  <th
                    key={col.key}
                    scope="col"
                    className={[
                      "px-3 py-2 text-left font-medium",
                      col.className ?? "",
                    ].join(" ")}
                  >
                    <button
                      type="button"
                      onClick={() => onToggleSort(col.key)}
                      className={[
                        "inline-flex items-center gap-1 hover:text-accent-blue",
                        active ? "text-accent-blue" : "",
                      ].join(" ")}
                      data-testid={`sort-${col.key}`}
                      aria-label={`Sort by ${col.label}`}
                    >
                      {col.label}
                      {active ? (
                        sort.dir === "asc" ? (
                          <ArrowUp size={10} />
                        ) : (
                          <ArrowDown size={10} />
                        )
                      ) : (
                        <ArrowUpDown size={10} className="opacity-40" />
                      )}
                    </button>
                  </th>
                );
              })}
              <th
                scope="col"
                className="px-3 py-2 text-left font-medium"
                aria-label="Actions"
              >
                Actions
              </th>
            </tr>
          </thead>
          <tbody className="divide-y divide-gray-100 dark:divide-neutral-700">
            {eventsQ.isLoading && (
              <tr>
                <td
                  colSpan={COLUMNS.length + 1}
                  className="px-3 py-6 text-center text-gray-400"
                >
                  <span className="inline-flex items-center gap-2">
                    <Loader2 size={12} className="animate-spin" />
                    Loading activity…
                  </span>
                </td>
              </tr>
            )}
            {!eventsQ.isLoading && visible.length === 0 && (
              <tr>
                <td
                  colSpan={COLUMNS.length + 1}
                  className="px-3 py-6 text-center text-gray-400"
                  data-testid="activity-empty"
                >
                  {events.length === 0
                    ? "No activity yet"
                    : "No events match the current filters"}
                </td>
              </tr>
            )}
            {!eventsQ.isLoading &&
              visible.map((ev) => (
                <ActivityRow
                  key={ev.id}
                  ev={ev}
                  onSelect={() => setSelected(ev)}
                  onOpenInTab={() => void onOpenInTab(ev)}
                />
              ))}
          </tbody>
        </table>
      </div>

      {hasMore && (
        <div className="text-center">
          <button
            type="button"
            onClick={() => setPageSize((n) => n + PAGE_SIZE)}
            className="px-3 py-1 text-xs rounded border border-gray-200 dark:border-neutral-700 hover:bg-gray-50 dark:hover:bg-neutral-700"
            data-testid="activity-load-more"
          >
            Load more ({sorted.length - pageSize} remaining)
          </button>
        </div>
      )}

      {selected && (
        <AuditEventDetailDialog
          ev={selected}
          onClose={() => setSelected(null)}
          onOpenInTab={() => void onOpenInTab(selected)}
        />
      )}
    </section>
  );
}

function ActivityRow({
  ev,
  onSelect,
  onOpenInTab,
}: {
  ev: AuditEventView;
  onSelect(): void;
  onOpenInTab(): void;
}) {
  const resource = ev.resource_id ?? "—";
  const hasNav = navAction(ev) !== null;
  return (
    <tr
      onClick={onSelect}
      className="cursor-pointer hover:bg-gray-50 dark:hover:bg-neutral-700/50"
      data-testid="activity-row"
    >
      <td className="px-3 py-2 font-mono text-[10px] whitespace-nowrap">
        {formatDateTime(ev.timestamp)}
      </td>
      <td className="px-3 py-2 font-mono text-[11px]">{ev.action}</td>
      <td className="px-3 py-2 truncate max-w-[10rem]">{ev.actor}</td>
      <td className="px-3 py-2 truncate max-w-[10rem]">
        {ev.resource_kind ?? "—"}
        {ev.resource_kind && ev.resource_id ? " · " : ""}
        {resource}
      </td>
      <td className="px-3 py-2 whitespace-nowrap">
        <span
          className={[
            "inline-flex items-center gap-1 px-1.5 py-0.5 rounded text-[10px] uppercase tracking-wider",
            ev.result === "Success"
              ? "bg-accent-green/15 text-accent-green"
              : "bg-accent-red/15 text-accent-red",
          ].join(" ")}
          data-result={ev.result}
        >
          <span
            className={[
              "inline-block w-1.5 h-1.5 rounded-full",
              ev.result === "Success" ? "bg-accent-green" : "bg-accent-red",
            ].join(" ")}
          />
          {ev.result}
        </span>
      </td>
      <td className="px-3 py-2 whitespace-nowrap text-right">
        {hasNav ? (
          <button
            type="button"
            onClick={(e) => {
              e.stopPropagation();
              onOpenInTab();
            }}
            className="inline-flex items-center gap-1 text-accent-blue hover:underline"
            title="Open in tab"
            aria-label={`Open ${summary(ev)} in tab`}
          >
            <ExternalLink size={11} />
            Open in tab
          </button>
        ) : (
          <span className="text-[10px] text-gray-400">—</span>
        )}
      </td>
    </tr>
  );
}

function AuditEventDetailDialog({
  ev,
  onClose,
  onOpenInTab,
}: {
  ev: AuditEventView;
  onClose(): void;
  onOpenInTab(): void;
}) {
  const target = navAction(ev);
  return (
    <div
      role="dialog"
      aria-modal="true"
      aria-label="Audit event detail"
      className="fixed inset-0 z-50 flex items-center justify-center bg-black/40"
      onClick={onClose}
      data-testid="audit-event-detail-dialog"
    >
      <div
        className="bg-white dark:bg-neutral-800 rounded-lg shadow-xl w-[640px] max-w-[95vw] max-h-[85vh] flex flex-col"
        onClick={(e) => e.stopPropagation()}
      >
        <header className="flex items-center justify-between px-4 py-3 border-b border-gray-200 dark:border-neutral-700">
          <h2 className="text-sm font-semibold">{summary(ev)}</h2>
          <button
            type="button"
            onClick={onClose}
            className="p-1 rounded text-gray-500 hover:bg-gray-100 dark:hover:bg-neutral-700"
            aria-label="Close detail"
          >
            <X size={14} />
          </button>
        </header>
        <div className="px-4 py-3 space-y-2 text-xs">
          <div className="flex flex-wrap items-center gap-2 text-[10px] text-gray-500 dark:text-neutral-400">
            <span className="font-mono px-1.5 py-0.5 rounded bg-gray-100 dark:bg-neutral-700">
              {ev.action}
            </span>
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
          <dl className="grid grid-cols-[7rem_1fr] gap-x-2 gap-y-1 text-[11px]">
            {detailRows(ev).map(([k, v]) => (
              <FragmentRow key={k} k={k} v={v} />
            ))}
          </dl>
        </div>
        <footer className="flex items-center justify-end gap-2 px-4 py-3 border-t border-gray-200 dark:border-neutral-700">
          <button
            type="button"
            onClick={onClose}
            className="px-3 py-1 text-xs rounded border border-gray-200 dark:border-neutral-700 hover:bg-gray-50 dark:hover:bg-neutral-700"
          >
            Close
          </button>
          {target && (
            <button
              type="button"
              onClick={() => {
                onOpenInTab();
                onClose();
              }}
              className="inline-flex items-center gap-1 px-3 py-1 text-xs rounded bg-accent-blue text-white hover:opacity-90"
            >
              <ExternalLink size={11} />
              {target.label}
            </button>
          )}
        </footer>
      </div>
    </div>
  );
}

function FragmentRow({ k, v }: { k: string; v: string }) {
  return (
    <>
      <dt className="text-gray-500 dark:text-neutral-400">{k}</dt>
      <dd className="font-mono text-gray-700 dark:text-neutral-200 break-all">
        {v}
      </dd>
    </>
  );
}

function detailRows(ev: AuditEventView): [string, string][] {
  return [
    ["resource_kind", ev.resource_kind ?? "—"],
    ["resource_id", ev.resource_id ?? "—"],
    ["application_id", ev.application_id === null ? "—" : String(ev.application_id)],
    ["correlation_id", ev.correlation_id ?? "—"],
    ["operation_id", ev.operation_id ?? "—"],
    ["nav_kind", ev.nav_kind ?? "—"],
    ["nav_resource_id", ev.nav_resource_id ?? "—"],
  ];
}

function compare(a: AuditEventView, b: AuditEventView, sort: SortState): number {
  const dir = sort.dir === "asc" ? 1 : -1;
  switch (sort.key) {
    case "timestamp":
      return (a.timestamp - b.timestamp) * dir;
    case "action":
      return a.action.localeCompare(b.action) * dir;
    case "actor":
      return a.actor.localeCompare(b.actor) * dir;
    case "resource": {
      const ar = `${a.resource_kind ?? ""}·${a.resource_id ?? ""}`;
      const br = `${b.resource_kind ?? ""}·${b.resource_id ?? ""}`;
      return ar.localeCompare(br) * dir;
    }
    case "result":
      return a.result.localeCompare(b.result) * dir;
  }
}

function formatDateTime(ts: number): string {
  const d = new Date(ts * 1000);
  const pad = (n: number) => String(n).padStart(2, "0");
  return `${d.getFullYear()}-${pad(d.getMonth() + 1)}-${pad(d.getDate())} ${pad(d.getHours())}:${pad(d.getMinutes())}:${pad(d.getSeconds())}`;
}

function parseDateStart(iso: string): number {
  const d = new Date(`${iso}T00:00:00`);
  return Math.floor(d.getTime() / 1000);
}

function parseDateEnd(iso: string): number {
  const d = new Date(`${iso}T23:59:59`);
  return Math.floor(d.getTime() / 1000);
}
