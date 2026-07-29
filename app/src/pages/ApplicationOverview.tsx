import { useEffect, useState } from "react";
import { LayoutDashboard, Workflow, Database, Download, Upload } from "lucide-react";

import { useApplications } from "../state/applicationsStore";
import type { ApplicationView, DisableReport, ImportReportView } from "../ipc/applications";
import {
  applicationExport,
  applicationImport,
} from "../ipc/applications";
import { auditQuery } from "../ipc/audit";
import type { AuditEventView } from "../ipc/audit";

interface Props {
  applicationId: number;
}

export function ApplicationOverview({ applicationId }: Props) {
  const applications = useApplications((s) => s.items);
  const refresh = useApplications((s) => s.refresh);
  const enableAction = useApplications((s) => s.enable);
  const disableAction = useApplications((s) => s.disable);
  const app = applications.find((a) => a.id === applicationId);

  const [events, setEvents] = useState<AuditEventView[]>([]);
  const [lifecycleState, setLifecycleState] = useState<
    | { kind: "idle" }
    | { kind: "busy" }
    | { kind: "enabled" }
    | { kind: "disabled"; report: DisableReport }
    | { kind: "error"; message: string }
  >({ kind: "idle" });

  const [exportPass, setExportPass] = useState("");
  const [exportPath, setExportPath] = useState("");
  const [importPass, setImportPass] = useState("");
  const [importPath, setImportPath] = useState("");
  const [exportState, setExportState] = useState<"idle" | "busy" | "done" | "error">("idle");
  const [importState, setImportState] = useState<
    { kind: "idle" } | { kind: "busy" } | { kind: "done"; report: ImportReportView } | { kind: "error"; message: string }
  >({ kind: "idle" });

  useEffect(() => {
    if (applications.length === 0) void refresh();
  }, [applications.length, refresh]);

  useEffect(() => {
    if (!app) return;
    let cancelled = false;
    void auditQuery(app.id, 25).then((xs) => {
      if (!cancelled) setEvents(xs);
    });
    return () => {
      cancelled = true;
    };
  }, [app]);

  if (!app) {
    return (
      <div className="space-y-4">
        <h1 className="text-xl font-semibold">Application</h1>
        <p className="text-xs text-gray-400">loading…</p>
      </div>
    );
  }

  const onExport = async () => {
    setExportState("busy");
    try {
      await applicationExport(app.name, exportPass, exportPath);
      setExportState("done");
    } catch (e) {
      setExportState("error");
    }
  };

  const onImport = async () => {
    setImportState({ kind: "busy" });
    try {
      const report = await applicationImport(importPath, importPass);
      setImportState({ kind: "done", report });
    } catch (e) {
      setImportState({ kind: "error", message: (e as Error).message });
    }
  };

  const onEnable = async () => {
    setLifecycleState({ kind: "busy" });
    try {
      await enableAction(app.id);
      setLifecycleState({ kind: "enabled" });
      await refresh();
    } catch (e) {
      setLifecycleState({ kind: "error", message: (e as Error).message });
    }
  };

  const onDisable = async () => {
    setLifecycleState({ kind: "busy" });
    try {
      const report = await disableAction(app.id);
      setLifecycleState({ kind: "disabled", report });
      await refresh();
    } catch (e) {
      setLifecycleState({ kind: "error", message: (e as Error).message });
    }
  };

  return (
    <div className="space-y-6">
      <header className="flex items-center justify-between">
        <h1 className="text-xl font-semibold">{app.name}</h1>
        <div className="flex items-center gap-2">
          {app.enabled ? (
            <button
              data-testid="disable-app"
              onClick={() => void onDisable()}
              disabled={lifecycleState.kind === "busy"}
              className="px-3 py-1 text-xs rounded border border-gray-200 dark:border-neutral-700 hover:bg-gray-50 dark:hover:bg-neutral-700 disabled:opacity-50"
            >
              Disable
            </button>
          ) : (
            <button
              data-testid="enable-app"
              onClick={() => void onEnable()}
              disabled={lifecycleState.kind === "busy"}
              className="px-3 py-1 text-xs rounded bg-accent-blue text-white border-transparent hover:bg-accent-blue/90 disabled:opacity-50"
            >
              Enable
            </button>
          )}
        </div>
      </header>

      {lifecycleState.kind === "enabled" && (
        <div data-testid="lifecycle-enabled" className="text-[11px] text-accent-green">
          Application enabled
        </div>
      )}
      {lifecycleState.kind === "disabled" && (
        <div
          data-testid="disable-summary"
          className="text-[11px] text-gray-600 dark:text-neutral-300 bg-white dark:bg-neutral-800 border border-gray-200 dark:border-neutral-700 rounded-md p-2"
        >
          snapshot taken_at={lifecycleState.report.snapshot.taken_at} · pipelines:{" "}
          {lifecycleState.report.pipelines.length} · datasources:{" "}
          {lifecycleState.report.datasources.length}
        </div>
      )}
      {lifecycleState.kind === "error" && (
        <div className="text-[11px] text-accent-red">{lifecycleState.message}</div>
      )}

      <div className="grid grid-cols-1 md:grid-cols-3 gap-3">
        <Tile icon={<LayoutDashboard size={14} />} label="Dashboard" sub="drag/resize grid · coming in a later slice" />
        <Tile icon={<Workflow size={14} />} label="Pipelines" sub="definitions + jobs · coming in a later slice" />
        <Tile icon={<Database size={14} />} label="Datasources" sub="managed providers · coming in a later slice" />
      </div>

      <div className="grid grid-cols-1 md:grid-cols-2 gap-3">
        <section className="bg-white dark:bg-neutral-800 rounded-lg border border-gray-200 dark:border-neutral-700 p-3">
          <div className="flex items-center gap-2 mb-2">
            <Download size={14} className="text-accent-blue" />
            <h2 className="text-sm font-semibold">Export</h2>
          </div>
          <div className="space-y-2 text-xs">
            <Field label="Passphrase">
              <input
                type="password"
                aria-label="passphrase export"
                value={exportPass}
                onChange={(e) => setExportPass(e.target.value)}
                placeholder="strong passphrase"
                className="flex-1 px-2 py-1 rounded border border-gray-200 dark:border-neutral-700 bg-white dark:bg-neutral-900"
              />
            </Field>
            <Field label="File path">
              <input
                aria-label="file path export"
                value={exportPath}
                onChange={(e) => setExportPath(e.target.value)}
                placeholder="/Users/me/alpha.bapp"
                className="flex-1 px-2 py-1 rounded border border-gray-200 dark:border-neutral-700 bg-white dark:bg-neutral-900 font-mono"
              />
            </Field>
            <div className="pt-1 flex items-center gap-2">
              <button
                type="button"
                onClick={() => void onExport()}
                disabled={exportState === "busy" || exportPass.length === 0 || exportPath.length === 0}
                className="px-3 py-1 rounded bg-accent-blue text-white hover:bg-accent-blue/90 disabled:opacity-50"
              >
                Export
              </button>
              {exportState === "done" && <span className="text-accent-green text-[11px]">exported</span>}
              {exportState === "error" && <span className="text-accent-red text-[11px]">failed</span>}
              {exportState === "busy" && <span className="text-gray-500 text-[11px]">exporting…</span>}
            </div>
          </div>
        </section>

        <section className="bg-white dark:bg-neutral-800 rounded-lg border border-gray-200 dark:border-neutral-700 p-3">
          <div className="flex items-center gap-2 mb-2">
            <Upload size={14} className="text-accent-green" />
            <h2 className="text-sm font-semibold">Import</h2>
          </div>
          <div className="space-y-2 text-xs">
            <Field label="Passphrase">
              <input
                type="password"
                aria-label="passphrase import"
                value={importPass}
                onChange={(e) => setImportPass(e.target.value)}
                placeholder="passphrase used at export"
                className="flex-1 px-2 py-1 rounded border border-gray-200 dark:border-neutral-700 bg-white dark:bg-neutral-900"
              />
            </Field>
            <Field label="File path">
              <input
                aria-label="file path import"
                value={importPath}
                onChange={(e) => setImportPath(e.target.value)}
                placeholder="/Users/me/alpha.bapp"
                className="flex-1 px-2 py-1 rounded border border-gray-200 dark:border-neutral-700 bg-white dark:bg-neutral-900 font-mono"
              />
            </Field>
            <div className="pt-1 flex items-center gap-2">
              <button
                type="button"
                onClick={() => void onImport()}
                disabled={importState.kind === "busy" || importPass.length === 0 || importPath.length === 0}
                className="px-3 py-1 rounded bg-accent-green text-white hover:bg-accent-green/90 disabled:opacity-50"
              >
                Import
              </button>
              {importState.kind === "busy" && <span className="text-gray-500 text-[11px]">importing…</span>}
              {importState.kind === "error" && (
                <span className="text-accent-red text-[11px]">{importState.message}</span>
              )}
            </div>
            {importState.kind === "done" && (
              <div data-testid="import-summary" className="text-[11px] text-gray-600 dark:text-neutral-300">
                created: {importState.report.created.join(", ") || "—"} ·{" "}
                skipped: {importState.report.skipped.join(", ") || "—"}
              </div>
            )}
          </div>
        </section>
      </div>

      <section>
        <h2 className="text-sm font-semibold text-gray-700 dark:text-neutral-200 mb-2">
          Recent activity ({events.length})
        </h2>
        {events.length === 0 ? (
          <p className="text-xs text-gray-400">no events yet</p>
        ) : (
          <ul className="bg-white dark:bg-neutral-800 rounded-lg border border-gray-200 dark:border-neutral-700 divide-y divide-gray-100 dark:divide-neutral-700">
            {events.slice(0, 10).map((e) => (
              <li key={e.id} className="px-4 py-2 text-xs flex items-center gap-2">
                <span
                  className={[
                    "inline-block w-1.5 h-1.5 rounded-full shrink-0",
                    e.result === "Success" ? "bg-accent-green" : "bg-accent-red",
                  ].join(" ")}
                />
                <span className="flex-1 truncate">{e.summary}</span>
                <span className="text-[10px] text-gray-400">{e.action}</span>
              </li>
            ))}
          </ul>
        )}
      </section>
    </div>
  );
}

function Tile({ icon, label, sub }: { icon: React.ReactNode; label: string; sub: string }) {
  return (
    <div className="bg-white dark:bg-neutral-800 rounded-lg border border-gray-200 dark:border-neutral-700 p-3 flex items-start gap-2">
      <div className="w-8 h-8 rounded bg-accent-blue/10 text-accent-blue flex items-center justify-center">
        {icon}
      </div>
      <div>
        <div className="text-sm font-medium">{label}</div>
        <div className="text-[10px] text-gray-400">{sub}</div>
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
      <label className="w-28 text-xs text-gray-600 dark:text-neutral-400">
        {label}
      </label>
      {children}
    </div>
  );
}

export function renderApplicationTab(app: ApplicationView | undefined) {
  if (!app) return null;
  return <ApplicationOverview applicationId={app.id} />;
}