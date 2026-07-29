import { useEffect, useState } from "react";
import {
  Hexagon,
  Plus,
  ChevronRight,
  ChevronDown,
  LayoutDashboard,
  Workflow,
  Database,
  Trash2,
  Power,
  PowerOff,
} from "lucide-react";

import { useApplications } from "../state/applicationsStore";
import { useTabs } from "../state/tabsStore";
import type { TabKind } from "../state/tabsStore";
import { useConnection } from "../state/connectionStore";
import { SearchBox } from "./SearchBox";
import type { SearchHit } from "../ipc/search";

function titleFor(kind: TabKind, resourceId: string | null): string {
  switch (kind) {
    case "cluster":
      return "Cluster";
    case "application":
      return resourceId ? `Application ${resourceId}` : "Application";
    case "application_pipelines":
      return resourceId ? `${resourceId} · Pipelines` : "Pipelines";
    case "application_datasources":
      return resourceId ? `${resourceId} · Datasources` : "Datasources";
    case "application_dashboard":
      return resourceId ? `${resourceId} · Dashboard` : "Dashboard";
    case "pipeline":
      return resourceId ? `Pipeline ${resourceId}` : "Pipeline";
    case "datasource":
      return resourceId ? `Datasource ${resourceId}` : "Datasource";
    case "pipeline_editor":
      return "New Pipeline";
  }
}

function matchesQuery(text: string, q: string): boolean {
  if (!q) return true;
  return text.toLowerCase().includes(q.toLowerCase());
}

export function NavTree() {
  const applications = useApplications((s) => s.items);
  const applicationsLoaded = useApplications((s) => s.loaded);
  const refreshApps = useApplications((s) => s.refresh);
  const createApp = useApplications((s) => s.create);
  const setEnabled = useApplications((s) => s.setEnabled);
  const deleteApp = useApplications((s) => s.delete);

  const openTab = useTabs((s) => s.open);
  const tabs = useTabs((s) => s.tabs);
  const activeId = useTabs((s) => s.activeId);
  const close = useTabs((s) => s.close);
  const pin = useTabs((s) => s.pin);
  const addr = useConnection((s) => s.addr);

  const [query, setQuery] = useState("");
  const [adding, setAdding] = useState(false);
  const [draftName, setDraftName] = useState("");
  const [expanded, setExpanded] = useState<Record<number, boolean>>({});

  useEffect(() => {
    if (!applicationsLoaded) {
      void refreshApps();
    }
  }, [applicationsLoaded, refreshApps]);

  const filtered = applications.filter((a) => matchesQuery(a.name, query));

  const onPickHit = async (hit: SearchHit) => {
    switch (hit.kind) {
      case "Pipeline":
        await openTab({
          kind: "pipeline",
          resourceId: hit.id,
          title: hit.title,
        });
        return;
      case "Datasource":
        await openTab({
          kind: "datasource",
          resourceId: hit.id,
          title: hit.title,
        });
        return;
      case "Application":
        await openTab({
          kind: "application",
          resourceId: hit.id,
          title: hit.title,
        });
        return;
      case "Dashboard":
        await openTab({
          kind: "application_dashboard",
          resourceId: hit.id,
          title: titleFor("application_dashboard", hit.id),
        });
        return;
      case "ClusterNode":
        await openTab({
          kind: "cluster",
          resourceId: null,
          title: "Cluster",
        });
        return;
    }
  };

  const onCreate = async () => {
    const name = draftName.trim();
    if (!name) return;
    await createApp(name);
    setDraftName("");
    setAdding(false);
  };

  return (
    <div className="h-full flex flex-col">
      <div className="flex items-center justify-between px-3 py-2 border-b border-gray-200 dark:border-neutral-700">
        <div className="flex items-center gap-2">
          <Hexagon size={14} className="text-accent-blue" />
          <span className="text-sm font-semibold">Bee</span>
        </div>
        <button
          aria-label="Add application"
          title="Add application"
          onClick={() => setAdding((s) => !s)}
          className="p-1 rounded text-gray-500 hover:bg-gray-100 dark:hover:bg-neutral-700"
        >
          <Plus size={14} />
        </button>
      </div>

      <div className="px-2 pt-2">
        <SearchBox query={query} onQueryChange={setQuery} onPick={onPickHit} addr={addr} />
      </div>

      <nav className="flex-1 overflow-y-auto py-2 px-1 space-y-0.5 text-xs">
        <button
          onClick={() => void openTab({ kind: "cluster", resourceId: null, title: "Cluster" })}
          className={[
            "w-full flex items-center gap-2 px-2 py-1.5 rounded",
            isActive(tabs, activeId, "cluster", null)
              ? "bg-accent-blue/10 text-accent-blue"
              : "text-gray-700 dark:text-neutral-200 hover:bg-gray-100 dark:hover:bg-neutral-700",
          ].join(" ")}
        >
          <Hexagon size={11} />
          <span>Cluster</span>
        </button>

        <div className="px-2 pt-3 pb-1 flex items-center justify-between text-[10px] font-semibold uppercase tracking-wider text-gray-500 dark:text-neutral-400">
          <span>Applications ({filtered.length})</span>
        </div>

        {adding && (
          <div className="px-2 pb-2">
            <input
              value={draftName}
              onChange={(e) => setDraftName(e.target.value)}
              onKeyDown={(e) => {
                if (e.key === "Enter") void onCreate();
                if (e.key === "Escape") {
                  setAdding(false);
                  setDraftName("");
                }
              }}
              placeholder="Application name"
              className="w-full px-2 py-1 text-xs rounded border border-gray-200 dark:border-neutral-700 bg-white dark:bg-neutral-800"
              autoFocus
            />
          </div>
        )}

        {filtered.length === 0 && !adding && (
          <p className="px-2 py-3 text-[11px] text-gray-400">
            {applications.length === 0
              ? "No applications yet — click + to add"
              : "no matches"}
          </p>
        )}

        {filtered.map((app) => {
          const isOpen = expanded[app.id] ?? true;
          const activeApp = isActive(tabs, activeId, "application", String(app.id));
          return (
            <div key={app.id}>
              <div
                className={[
                  "group flex items-center gap-1 px-2 py-1 rounded cursor-pointer",
                  activeApp && !isOpen
                    ? "bg-accent-blue/10 text-accent-blue"
                    : "text-gray-700 dark:text-neutral-200 hover:bg-gray-100 dark:hover:bg-neutral-700",
                ].join(" ")}
              >
                <button
                  aria-label={isOpen ? "Collapse" : "Expand"}
                  onClick={() => setExpanded((s) => ({ ...s, [app.id]: !isOpen }))}
                  className="p-0.5 text-gray-400"
                >
                  {isOpen ? <ChevronDown size={10} /> : <ChevronRight size={10} />}
                </button>
                <button
                  onClick={() =>
                    void openTab({
                      kind: "application",
                      resourceId: String(app.id),
                      title: app.name,
                    })
                  }
                  className="flex-1 text-left truncate"
                  title={app.name}
                >
                  {app.name}
                  {!app.enabled && (
                    <span className="ml-1 text-[9px] text-gray-400">·paused</span>
                  )}
                </button>
                <button
                  aria-label={app.enabled ? "Disable" : "Enable"}
                  title={app.enabled ? "Disable" : "Enable"}
                  onClick={() => void setEnabled(app.id, !app.enabled)}
                  className="opacity-0 group-hover:opacity-100 p-0.5 text-gray-400 hover:text-accent-blue"
                >
                  {app.enabled ? <Power size={10} /> : <PowerOff size={10} />}
                </button>
                <button
                  aria-label="Delete application"
                  title="Delete application"
                  onClick={() => {
                    if (confirm(`Delete application "${app.name}"?`))
                      void deleteApp(app.id);
                  }}
                  className="opacity-0 group-hover:opacity-100 p-0.5 text-gray-400 hover:text-accent-red"
                >
                  <Trash2 size={10} />
                </button>
              </div>
              {isOpen && (
                <div className="ml-4 space-y-0.5">
                  <ChildRow
                    icon={<LayoutDashboard size={10} />}
                    label="Dashboard"
                    active={isActive(
                      tabs,
                      activeId,
                      "application_dashboard",
                      String(app.id),
                    )}
                    onClick={() =>
                      void openTab({
                        kind: "application_dashboard",
                        resourceId: String(app.id),
                        title: titleFor("application_dashboard", String(app.id)),
                      })
                    }
                  />
                  <ChildRow
                    icon={<Workflow size={10} />}
                    label="Pipelines"
                    active={isActive(
                      tabs,
                      activeId,
                      "application_pipelines",
                      String(app.id),
                    )}
                    onClick={() =>
                      void openTab({
                        kind: "application_pipelines",
                        resourceId: String(app.id),
                        title: titleFor("application_pipelines", String(app.id)),
                      })
                    }
                  />
                  <ChildRow
                    icon={<Database size={10} />}
                    label="Datasources"
                    active={isActive(
                      tabs,
                      activeId,
                      "application_datasources",
                      String(app.id),
                    )}
                    onClick={() =>
                      void openTab({
                        kind: "application_datasources",
                        resourceId: String(app.id),
                        title: titleFor("application_datasources", String(app.id)),
                      })
                    }
                  />
                </div>
              )}
            </div>
          );
        })}
      </nav>

      <div className="border-t border-gray-200 dark:border-neutral-700 p-2 text-[10px] text-gray-500 dark:text-neutral-400">
        <button
          onClick={() => {
            tabs
              .filter((t) => t.id !== activeId)
              .forEach((t) => void close(t.id));
          }}
          className="w-full px-2 py-1 text-left text-xs rounded hover:bg-gray-100 dark:hover:bg-neutral-700"
        >
          Close other tabs
        </button>
        <button
          onClick={() => tabs.forEach((t) => void pin(t.id, !t.pinned))}
          className="w-full px-2 py-1 text-left text-xs rounded hover:bg-gray-100 dark:hover:bg-neutral-700"
        >
          Toggle pinned
        </button>
      </div>
    </div>
  );
}

function ChildRow({
  icon,
  label,
  active,
  onClick,
}: {
  icon: React.ReactNode;
  label: string;
  active: boolean;
  onClick: () => void;
}) {
  return (
    <button
      onClick={onClick}
      className={[
        "w-full flex items-center gap-1.5 px-2 py-1 rounded",
        active
          ? "bg-accent-blue/10 text-accent-blue"
          : "text-gray-600 dark:text-neutral-300 hover:bg-gray-100 dark:hover:bg-neutral-700",
      ].join(" ")}
    >
      {icon}
      <span>{label}</span>
    </button>
  );
}

function isActive(
  tabs: { id: number; kind: string; resource_id: string | null }[],
  activeId: number | null,
  kind: string,
  resourceId: string | null,
): boolean {
  if (activeId === null) return false;
  const tab = tabs.find((t) => t.id === activeId);
  if (!tab) return false;
  if (tab.kind !== kind) return false;
  return (tab.resource_id ?? null) === resourceId;
}
