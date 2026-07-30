import { useEffect, useState, type ReactNode } from "react";
import { useQueryClient } from "@tanstack/react-query";
import { Cog, Moon, Sun, RefreshCw, X, Pin, PinOff } from "lucide-react";

import { NavTree } from "./NavTree";
import { SettingsModal } from "./SettingsModal";
import { ContextMenu, type ContextMenuItem } from "./ContextMenu";
import { ActivityBar } from "./ActivityBar";
import { ConnectionStatus } from "./ConnectionStatus";
import { useUi } from "../state/store";
import { useTabs, type TabKind } from "../state/tabsStore";
import { useApplications } from "../state/applicationsStore";
import { useConnection } from "../state/connectionStore";
import { ClusterDashboard } from "../pages/ClusterDashboard";
import { PipelinesPage } from "../pages/PipelinesPage";
import { PipelineDetail } from "../pages/PipelineDetail";
import { PipelineEditor } from "../pages/PipelineEditor";
import { DataSources } from "../pages/DataSources";
import { ApplicationOverview } from "../pages/ApplicationOverview";
import { DashboardPage } from "../pages/DashboardPage";

const TAB_KINDS: ReadonlySet<TabKind> = new Set([
  "cluster",
  "application",
  "application_pipelines",
  "application_datasources",
  "application_dashboard",
  "pipeline",
  "datasource",
  "pipeline_editor",
]);

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

export function AppShell({ children }: { children?: ReactNode }) {
  const settingsOpen = useUi((s) => s.settingsOpen);
  const closeSettings = useUi((s) => s.closeSettings);
  const openSettings = useUi((s) => s.openSettings);
  const theme = useUi((s) => s.theme);
  const toggleTheme = useUi((s) => s.toggleTheme);

  const openTab = useTabs((s) => s.open);
  const hydrateTabs = useTabs((s) => s.hydrate);
  const hydrateApps = useApplications((s) => s.refresh);
  const qc = useQueryClient();
  const addr = useConnection((s) => s.addr);

  useEffect(() => {
    void hydrateTabs();
    void hydrateApps();
  }, [hydrateTabs, hydrateApps]);

  const onRefresh = () => {
    qc.invalidateQueries({ queryKey: ["cluster", addr] });
    qc.invalidateQueries({ queryKey: ["jobs", addr] });
    qc.invalidateQueries({ queryKey: ["application-jobs"] });
    qc.invalidateQueries({ queryKey: ["dashboard-metrics"] });
  };

  const onActivityNavigate = (kind: string, resourceId: string | null) => {
    if (kind === "settings.connection") {
      openSettings("connection");
      return;
    }
    if (TAB_KINDS.has(kind as TabKind)) {
      void openTab({
        kind: kind as TabKind,
        resourceId,
        title: titleFor(kind as TabKind, resourceId),
      });
    }
  };

  return (
    <div className="h-full flex flex-col bg-gray-50 dark:bg-neutral-900 text-gray-900 dark:text-neutral-100">
      <header className="flex items-center gap-2 h-10 px-3 bg-white dark:bg-neutral-800 border-b border-gray-200 dark:border-neutral-700 text-xs">
        <span
          className="font-semibold text-accent-blue tracking-wide"
          data-testid="brand-label"
        >
          Bee
        </span>
        <div className="flex-1" />
        <button
          onClick={onRefresh}
          title="Refresh"
          aria-label="Refresh"
          className="p-1.5 rounded text-gray-500 hover:bg-gray-100 dark:hover:bg-neutral-700"
        >
          <RefreshCw size={14} />
        </button>
        <button
          onClick={() => openSettings()}
          className="p-1.5 rounded text-gray-500 hover:bg-gray-100 dark:hover:bg-neutral-700"
          title="Open settings"
          aria-label="Open settings"
        >
          <Cog size={14} />
        </button>
        <button
          onClick={toggleTheme}
          title={theme === "light" ? "Switch to Dark" : "Switch to Light"}
          aria-label="Toggle theme"
          className="p-1 rounded text-gray-500 hover:bg-gray-100 dark:hover:bg-neutral-700"
        >
          {theme === "light" ? <Moon size={14} /> : <Sun size={14} />}
        </button>
      </header>
      <div className="flex-1 flex overflow-hidden">
        <aside className="w-64 border-r border-gray-200 dark:border-neutral-700 bg-white dark:bg-neutral-800">
          <NavTree />
        </aside>
        <main className="flex-1 flex flex-col overflow-hidden">
          <PageTabs />
          {children && <div className="flex-1 overflow-auto p-6">{children}</div>}
        </main>
      </div>
      <StatusBar onActivityNavigate={onActivityNavigate} />
      {settingsOpen && <SettingsModal open={settingsOpen} onClose={closeSettings} />}
    </div>
  );
}

interface StatusBarProps {
  onActivityNavigate(kind: string, resourceId: string | null): void;
}

function StatusBar({ onActivityNavigate }: StatusBarProps) {
  const theme = useUi((s) => s.theme);
  const toggleTheme = useUi((s) => s.toggleTheme);
  const status = useConnection((s) => s.status);

  return (
    <footer className="flex items-center gap-4 px-3 h-7 text-[10px] text-gray-500 dark:text-neutral-400 bg-white dark:bg-neutral-800 border-t border-gray-200 dark:border-neutral-700">
      <span className="font-medium text-gray-700 dark:text-neutral-200">Bee Client v0.1.0</span>
      <ConnectionStatus onOpenSettings={() => useUi.getState().openSettings()} />
      <ActivityBar navigate={onActivityNavigate} />
      <span
        className="text-[10px] text-gray-400 font-mono"
        data-testid="footer-cluster-detail"
      >
        {status.kind === "Connected"
          ? "cluster reachable"
          : status.kind === "Connecting"
            ? "connecting…"
            : status.kind === "Error"
              ? `error: ${status.reason}`
              : "no cluster"}
      </span>
      <div className="flex-1" />
      <button
        onClick={toggleTheme}
        title={theme === "light" ? "Switch to Dark" : "Switch to Light"}
        className="p-1 rounded text-gray-500 hover:bg-gray-100 dark:hover:bg-neutral-700"
      >
        {theme === "light" ? <Moon size={12} /> : <Sun size={12} />}
      </button>
    </footer>
  );
}

interface MenuState {
  tabId: number;
  x: number;
  y: number;
}

function PageTabs() {
  const tabs = useTabs((s) => s.tabs);
  const activeId = useTabs((s) => s.activeId);
  const setActive = useTabs((s) => s.setActive);
  const close = useTabs((s) => s.close);
  const closeOthers = useTabs((s) => s.closeOthers);
  const closeRight = useTabs((s) => s.closeRight);
  const pin = useTabs((s) => s.pin);
  const applications = useApplications((s) => s.items);

  const [menu, setMenu] = useState<MenuState | null>(null);

  const active = tabs.find((t) => t.id === activeId);

  const body = (() => {
    if (!active) return null;
    switch (active.kind) {
      case "cluster":
        return <ClusterDashboard />;
      case "application": {
        const id = Number(active.resource_id);
        const app = applications.find((a) => a.id === id);
        if (!app) return <p className="text-xs text-gray-400">application not found</p>;
        return <ApplicationOverview applicationId={app.id} />;
      }
      case "application_pipelines":
        return <PipelinesPage />;
      case "application_datasources":
        return <DataSources />;
      case "application_dashboard": {
        const id = Number(active.resource_id);
        if (!Number.isFinite(id)) {
          return <p className="text-xs text-gray-400">invalid application id</p>;
        }
        return <DashboardPage applicationId={id} />;
      }
      case "pipeline": {
        const id = Number(active.resource_id);
        if (!Number.isFinite(id)) {
          return <p className="text-xs text-gray-400">invalid pipeline id</p>;
        }
        return <PipelineDetail pipelineId={id} />;
      }
      case "pipeline_editor":
        return <PipelineEditor />;
      case "datasource":
        return <p className="text-xs text-gray-400">datasource detail · coming in a later slice</p>;
    }
  })();

  const menuItems = (tab: { id: number; pinned: boolean }): ContextMenuItem[] => {
    const idx = tabs.findIndex((t) => t.id === tab.id);
    const hasRight = idx >= 0 && idx < tabs.length - 1;
    return [
      { id: "close", label: "Close", onSelect: () => void close(tab.id) },
      {
        id: "close-others",
        label: "Close Others",
        onSelect: () => void closeOthers(tab.id),
        disabled: tabs.length <= 1,
      },
      {
        id: "close-right",
        label: "Close to the Right",
        onSelect: () => void closeRight(tab.id),
        disabled: !hasRight,
      },
      {
        id: "pin",
        label: tab.pinned ? "Unpin" : "Pin",
        onSelect: () => void pin(tab.id, !tab.pinned),
      },
    ];
  };

  return (
    <div className="flex-1 flex flex-col overflow-hidden">
      <div
        role="tablist"
        className="flex items-end gap-1 px-2 pt-2 bg-white dark:bg-neutral-800 border-b border-gray-200 dark:border-neutral-700 overflow-x-auto"
      >
        {tabs.map((t) => {
          const isActive = t.id === activeId;
          return (
            <div
              key={t.id}
              role="tab"
              aria-selected={isActive}
              onClick={() => void setActive(t.id)}
              onContextMenu={(e) => {
                e.preventDefault();
                setMenu({ tabId: t.id, x: e.clientX, y: e.clientY });
              }}
              className={[
                "group flex items-center gap-1.5 px-3 h-8 rounded-t-md text-xs cursor-pointer",
                isActive
                  ? "bg-gray-50 dark:bg-neutral-900 text-gray-900 dark:text-neutral-100"
                  : "text-gray-600 dark:text-neutral-300 hover:bg-gray-100 dark:hover:bg-neutral-700",
              ].join(" ")}
            >
              <span className="truncate max-w-[14rem]">{t.title}</span>
              <button
                aria-label={t.pinned ? "Unpin tab" : "Pin tab"}
                title={t.pinned ? "Unpin" : "Pin"}
                onClick={(e) => {
                  e.stopPropagation();
                  void pin(t.id, !t.pinned);
                }}
                className="opacity-0 group-hover:opacity-100 p-0.5 rounded text-gray-400 hover:text-accent-blue"
              >
                {t.pinned ? <PinOff size={10} /> : <Pin size={10} />}
              </button>
              <button
                aria-label={`Close ${t.title}`}
                title="Close"
                onClick={(e) => {
                  e.stopPropagation();
                  void close(t.id);
                }}
                className="opacity-0 group-hover:opacity-100 p-0.5 rounded text-gray-400 hover:text-accent-red"
              >
                <X size={10} />
              </button>
            </div>
          );
        })}
      </div>
      <div className="flex-1 overflow-auto p-6">{body}</div>
      {menu && (
        <ContextMenu
          open
          x={menu.x}
          y={menu.y}
          items={(() => {
            const tab = tabs.find((t) => t.id === menu.tabId);
            return tab ? menuItems(tab) : [];
          })()}
          onClose={() => setMenu(null)}
        />
      )}
    </div>
  );
}
