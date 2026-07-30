import { useEffect, type ReactNode } from "react";
import { Cog, Moon, Sun, X, Pin, PinOff } from "lucide-react";

import { NavTree } from "./NavTree";
import { StatusBar } from "./StatusBar";
import { SettingsModal } from "./SettingsModal";
import { useUi } from "../state/store";
import { useTabs } from "../state/tabsStore";
import { useApplications } from "../state/applicationsStore";
import { ClusterDashboard } from "../pages/ClusterDashboard";
import { PipelinesPage } from "../pages/PipelinesPage";
import { PipelineDetail } from "../pages/PipelineDetail";
import { PipelineEditor } from "../pages/PipelineEditor";
import { DataSources } from "../pages/DataSources";
import { ApplicationOverview } from "../pages/ApplicationOverview";
import { DashboardPage } from "../pages/DashboardPage";

export function AppShell({ children }: { children?: ReactNode }) {
  const settingsOpen = useUi((s) => s.settingsOpen);
  const closeSettings = useUi((s) => s.closeSettings);
  const openSettings = useUi((s) => s.openSettings);
  const theme = useUi((s) => s.theme);
  const toggleTheme = useUi((s) => s.toggleTheme);

  const hydrateTabs = useTabs((s) => s.hydrate);
  const hydrateApps = useApplications((s) => s.refresh);

  useEffect(() => {
    void hydrateTabs();
    void hydrateApps();
  }, [hydrateTabs, hydrateApps]);

  return (
    <div className="h-full flex flex-col bg-gray-50 dark:bg-neutral-900 text-gray-900 dark:text-neutral-100">
      <header className="flex items-center h-9 px-3 bg-white dark:bg-neutral-800 border-b border-gray-200 dark:border-neutral-700 text-xs">
        <div className="flex-1" />
        <button
          onClick={openSettings}
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
          className="ml-1 p-1.5 rounded text-gray-500 hover:bg-gray-100 dark:hover:bg-neutral-700"
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
          <div className="flex-1 overflow-auto p-6">{children}</div>
        </main>
      </div>
      <StatusBar />
      {settingsOpen && <SettingsModal open={settingsOpen} onClose={closeSettings} />}
    </div>
  );
}

function PageTabs() {
  const tabs = useTabs((s) => s.tabs);
  const activeId = useTabs((s) => s.activeId);
  const setActive = useTabs((s) => s.setActive);
  const close = useTabs((s) => s.close);
  const pin = useTabs((s) => s.pin);
  const applications = useApplications((s) => s.items);

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
    </div>
  );
}