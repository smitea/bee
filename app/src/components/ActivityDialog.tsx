import { useEffect, useState } from "react";
import { X, RefreshCw } from "lucide-react";

import { useAudit, navTarget, summary } from "../state/auditStore";
import { useTabs } from "../state/tabsStore";
import type { AuditEventView } from "../ipc/audit";

export function ActivityDialog({ onClose }: { onClose: () => void }) {
  const events = useAudit((s) => s.events);
  const refresh = useAudit((s) => s.refresh);
  const openTab = useTabs((s) => s.open);

  const [filter, setFilter] = useState<"all" | "errors">("all");

  useEffect(() => {
    const onKey = (e: KeyboardEvent) => {
      if (e.key === "Escape") onClose();
    };
    window.addEventListener("keydown", onKey);
    return () => window.removeEventListener("keydown", onKey);
  }, [onClose]);

  const filtered = filter === "errors"
    ? events.filter((e) => e.result !== "Success")
    : events;

  return (
    <div
      className="fixed inset-0 z-50 flex items-center justify-center bg-black/40"
      role="dialog"
      aria-modal="true"
      aria-label="Activity"
      onClick={onClose}
    >
      <div
        className="bg-white dark:bg-neutral-800 rounded-lg shadow-xl w-[680px] max-w-[95vw] max-h-[80vh] flex flex-col"
        onClick={(e) => e.stopPropagation()}
      >
        <header className="flex items-center justify-between px-4 py-3 border-b border-gray-200 dark:border-neutral-700">
          <h2 className="text-sm font-semibold">Activity</h2>
          <div className="flex items-center gap-2">
            <select
              value={filter}
              onChange={(e) => setFilter(e.target.value as "all" | "errors")}
              className="text-xs px-2 py-1 rounded border border-gray-200 dark:border-neutral-700 bg-white dark:bg-neutral-900"
            >
              <option value="all">All</option>
              <option value="errors">Errors only</option>
            </select>
            <button
              onClick={() => void refresh(100)}
              className="p-1 rounded text-gray-500 hover:bg-gray-100 dark:hover:bg-neutral-700"
              title="Refresh"
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
        <div className="flex-1 overflow-y-auto px-2 py-2">
          {filtered.length === 0 ? (
            <p className="text-xs text-gray-400 px-2 py-6 text-center">
              {events.length === 0 ? "No activity yet" : "No matching events"}
            </p>
          ) : (
            <ul className="divide-y divide-gray-100 dark:divide-neutral-700">
              {filtered.map((e) => (
                <Row
                  key={e.id}
                  ev={e}
                  onNavigate={() => {
                    const target = navTarget(e);
                    if (!target) return;
                    void openTab({
                      kind: target.kind as never,
                      resourceId: target.resourceId,
                      title: e.summary,
                    });
                    onClose();
                  }}
                />
              ))}
            </ul>
          )}
        </div>
      </div>
    </div>
  );
}

function Row({ ev, onNavigate }: { ev: AuditEventView; onNavigate(): void }) {
  const target = navTarget(ev);
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
      <div className="mt-1 pl-3.5 flex items-center gap-2 text-[10px] text-gray-500 dark:text-neutral-400">
        <span>{ev.action}</span>
        <span>·</span>
        <span>{ev.actor}</span>
        {target && (
          <>
            <span>·</span>
            <button
              onClick={onNavigate}
              className="text-accent-blue hover:underline"
              title="Go to source"
            >
              Go to {target.kind}
            </button>
          </>
        )}
      </div>
    </li>
  );
}

function formatTime(ts: number): string {
  const d = new Date(ts * 1000);
  const pad = (n: number) => String(n).padStart(2, "0");
  return `${pad(d.getHours())}:${pad(d.getMinutes())}:${pad(d.getSeconds())}`;
}