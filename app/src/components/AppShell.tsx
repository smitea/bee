import { useState, type ReactNode } from "react";
import {
  Gauge,
  Database,
  Workflow,
  Settings as Cog,
  Moon,
  Sun,
  Plus,
  Search,
  Sparkles,
  ChevronRight,
  Server,

  Trash2,
  RefreshCcw,
  type LucideIcon,
} from "lucide-react";
import { useStore, type Tab } from "../state/store";
import { tooltipLabelForTab } from "../tooltip";

// ------------------------------------------------------------
// Saved connections (MongoDB Compass-style sidebar items)
// Persisted in localStorage, the same as the Tauri AdminServer addr.
// ------------------------------------------------------------
interface SavedConnection {
  id: string;
  label: string;
  addr: string;
  lastConnectedAtMs: number;
}

function loadConnections(): SavedConnection[] {
  if (typeof localStorage === "undefined") return [];
  try {
    return JSON.parse(localStorage.getItem("bee-gui.connections") || "[]");
  } catch {
    return [];
  }
}
function saveConnections(cs: SavedConnection[]) {
  localStorage.setItem("bee-gui.connections", JSON.stringify(cs));
}

export function AppShell({ children }: { children: ReactNode }) {
  const tab = useStore((s) => s.tab);
  const setTab = useStore((s) => s.setTab);
  const theme = useStore((s) => s.theme);
  const toggleTheme = useStore((s) => s.toggleTheme);
  const addr = useStore((s) => s.addr);
  const setAddr = useStore((s) => s.setAddr);
  const [connections, setConnections] = useState<SavedConnection[]>(
    loadConnections,
  );
  const [connSearch, setConnSearch] = useState("");
  const [showAdd, setShowAdd] = useState(false);
  const [newLabel, setNewLabel] = useState("");
  const [newAddr, setNewAddr] = useState("");

  // Keep the active connection's addr in the sidebar's "current" slot
  // and persist it in localStorage on every change.
  const ensureActive = (next: SavedConnection[]) => {
    if (!next.find((c) => c.addr === addr)) {
      // implicit "current" connection — show at top
    }
    saveConnections(next);
    setConnections(next);
  };

  const addConnection = () => {
    if (!newAddr.trim()) return;
    const id = `${Date.now()}-${Math.random().toString(36).slice(2, 8)}`;
    const next = [
      ...connections,
      {
        id,
        label: newLabel.trim() || newAddr.trim(),
        addr: newAddr.trim(),
        lastConnectedAtMs: Date.now(),
      },
    ];
    ensureActive(next);
    setNewLabel("");
    setNewAddr("");
    setShowAdd(false);
  };

  const removeConnection = (id: string) => {
    const next = connections.filter((c) => c.id !== id);
    ensureActive(next);
  };

  const filtered = connections.filter(
    (c) =>
      connSearch === "" ||
      c.label.toLowerCase().includes(connSearch.toLowerCase()) ||
      c.addr.toLowerCase().includes(connSearch.toLowerCase()),
  );

  return (
    <div className="h-full flex flex-col bg-gray-50 dark:bg-neutral-900 text-gray-900 dark:text-neutral-100">
      {/* ---- Top tab bar (Compass-style: Welcome / My Queries / Data Modeling ...) ---- */}
      <header className="flex items-center h-9 px-3 bg-white dark:bg-neutral-800 border-b border-gray-200 dark:border-neutral-700 text-xs">
        <div className="flex items-center gap-1">
          <Tab id="dashboard" Icon={Gauge} label="Welcome" active={tab === "dashboard"} onSelect={setTab} />
          <Tab id="dataSources" Icon={Database} label="Data Sources" active={tab === "dataSources"} onSelect={setTab} />
          <Tab id="pipelines" Icon={Workflow} label="Pipelines" active={tab === "pipelines"} onSelect={setTab} />
          <Tab id="settings" Icon={Cog} label="Settings" active={tab === "settings"} onSelect={setTab} />
          <button
            className="ml-1 p-1 rounded text-gray-400 hover:bg-gray-100 dark:hover:bg-neutral-700"
            title="New tab"
          >
            <Plus size={12} />
          </button>
        </div>
        <div className="flex-1" />
        <button
          onClick={toggleTheme}
          title={theme === "light" ? "Switch to Dark" : "Switch to Light"}
          className="p-1.5 rounded text-gray-500 hover:bg-gray-100 dark:hover:bg-neutral-700"
        >
          {theme === "light" ? <Moon size={14} /> : <Sun size={14} />}
        </button>
      </header>

      {/* ---- Body: sidebar + main ---- */}
      <div className="flex-1 flex overflow-hidden">
        {/* Sidebar */}
        <aside className="w-60 bg-white dark:bg-neutral-800 border-r border-gray-200 dark:border-neutral-700 flex flex-col">
          <SidebarHeader onAddClick={() => setShowAdd(true)} />
          <div className="px-2 pt-1">
            <div className="relative">
              <Search
                size={12}
                className="absolute left-2 top-1/2 -translate-y-1/2 text-gray-400"
              />
              <input
                value={connSearch}
                onChange={(e) => setConnSearch(e.target.value)}
                placeholder="Search connections"
                className="w-full pl-7 pr-2 py-1.5 text-xs bg-gray-100 dark:bg-neutral-700 rounded border-0 focus:outline-none focus:ring-1 focus:ring-accent-blue"
              />
            </div>
          </div>

          <div className="px-3 pt-3 pb-1 flex items-center justify-between text-[10px] font-semibold uppercase tracking-wider text-gray-500 dark:text-neutral-400">
            <span>Connections ({connections.length})</span>
          </div>
          <div className="flex-1 overflow-y-auto px-1 pb-2 space-y-0.5">
            {filtered.length === 0 ? (
              <p className="px-2 py-3 text-[11px] text-gray-400">
                {connections.length === 0
                  ? "no connections yet — click + to add"
                  : "no matches"}
              </p>
            ) : (
              filtered.map((c) => (
                <ConnectionRow
                  key={c.id}
                  c={c}
                  active={c.addr === addr}
                  onClick={() => setAddr(c.addr)}
                  onRemove={() => removeConnection(c.id)}
                />
              ))
            )}
          </div>

          {showAdd && (
            <div className="border-t border-gray-200 dark:border-neutral-700 p-3 space-y-2 bg-gray-50 dark:bg-neutral-900">
              <input
                value={newLabel}
                onChange={(e) => setNewLabel(e.target.value)}
                placeholder="Label (e.g. dev-cluster)"
                className="w-full px-2 py-1 text-xs rounded border border-gray-200 dark:border-neutral-700 bg-white dark:bg-neutral-800"
              />
              <input
                value={newAddr}
                onChange={(e) => setNewAddr(e.target.value)}
                placeholder="127.0.0.1:8702"
                className="w-full px-2 py-1 text-xs rounded border border-gray-200 dark:border-neutral-700 bg-white dark:bg-neutral-800 font-mono"
              />
              <div className="flex gap-2">
                <button
                  onClick={addConnection}
                  className="flex-1 px-2 py-1 text-xs rounded bg-accent-blue text-white hover:bg-accent-blue/90"
                >
                  Save
                </button>
                <button
                  onClick={() => setShowAdd(false)}
                  className="px-2 py-1 text-xs rounded border border-gray-200 dark:border-neutral-700"
                >
                  Cancel
                </button>
              </div>
            </div>
          )}

          <SidebarFooter addr={addr} />
        </aside>

        {/* Main content */}
        <main className="flex-1 flex flex-col overflow-hidden">
          <TopToolbar />
          <div className="flex-1 overflow-auto p-6">{children}</div>
        </main>
      </div>

      {/* ---- Bottom status bar ---- */}
      <footer className="flex items-center gap-4 px-3 h-6 text-[10px] text-gray-500 dark:text-neutral-400 bg-white dark:bg-neutral-800 border-t border-gray-200 dark:border-neutral-700">
        <span className="font-medium text-gray-700 dark:text-neutral-200">
          bee-gui v0.1.0 (Tauri)
        </span>
        <span>addr: {addr}</span>
        <span>theme: {theme}</span>
        <div className="flex-1" />
        <span>Compass-inspired layout</span>
      </footer>
    </div>
  );
}

function Tab({
  id,
  Icon,
  label,
  active,
  onSelect,
}: {
  id: Tab;
  Icon: LucideIcon;
  label: string;
  active: boolean;
  onSelect: (t: Tab) => void;
}) {
  return (
    <HoverTooltip label={tooltipLabelForTab(id)}>
      <button
        onClick={() => onSelect(id)}
        className={[
          "flex items-center gap-1.5 px-3 h-7 rounded-t-md text-xs transition-colors",
          active
            ? "bg-accent-blue text-white"
            : "text-gray-600 dark:text-neutral-300 hover:bg-gray-100 dark:hover:bg-neutral-700",
        ].join(" ")}
      >
        <Icon size={12} />
        {label}
      </button>
    </HoverTooltip>
  );
}

function ConnectionRow({
  c,
  active,
  onClick,
  onRemove,
}: {
  c: SavedConnection;
  active: boolean;
  onClick: () => void;
  onRemove: () => void;
}) {
  return (
    <div
      onClick={onClick}
      className={[
        "group flex items-center gap-2 px-2 py-1.5 rounded-md cursor-pointer",
        active
          ? "bg-accent-blue/10 text-accent-blue"
          : "text-gray-700 dark:text-neutral-200 hover:bg-gray-100 dark:hover:bg-neutral-700",
      ].join(" ")}
    >
      <Server size={12} className="shrink-0" />
      <div className="flex-1 min-w-0">
        <div className="text-xs font-medium truncate">{c.label}</div>
        <div className="text-[10px] text-gray-500 dark:text-neutral-400 font-mono truncate">
          {c.addr}
        </div>
      </div>
      <button
        onClick={(e) => {
          e.stopPropagation();
          if (confirm(`Remove connection "${c.label}"?`)) onRemove();
        }}
        className="opacity-0 group-hover:opacity-100 p-1 rounded text-gray-400 hover:text-red-500"
        title="Remove"
      >
        <Trash2 size={11} />
      </button>
    </div>
  );
}

function SidebarHeader({ onAddClick }: { onAddClick: () => void }) {
  return (
    <div className="flex items-center justify-between px-3 py-2 border-b border-gray-200 dark:border-neutral-700">
      <span className="text-sm font-semibold">Compass</span>
      <button
        onClick={onAddClick}
        className="p-1 rounded text-gray-500 hover:bg-gray-100 dark:hover:bg-neutral-700"
        title="Add connection"
      >
        <Plus size={14} />
      </button>
    </div>
  );
}

function SidebarFooter({ addr }: { addr: string }) {
  return (
    <div className="border-t border-gray-200 dark:border-neutral-700 p-3 text-[10px] text-gray-500 dark:text-neutral-400">
      <div className="flex items-center gap-1.5">
        <span className="w-1.5 h-1.5 rounded-full bg-accent-red" />
        <span className="truncate">active: {addr}</span>
      </div>
    </div>
  );
}

function TopToolbar() {
  return (
    <div className="flex items-center gap-2 px-4 py-2 bg-white dark:bg-neutral-800 border-b border-gray-200 dark:border-neutral-700">
      <div className="flex-1 relative max-w-md">
        <Search
          size={12}
          className="absolute left-3 top-1/2 -translate-y-1/2 text-gray-400"
        />
        <input
          placeholder="Search"
          className="w-full pl-8 pr-3 py-1.5 text-xs bg-gray-100 dark:bg-neutral-700 rounded border-0 focus:outline-none focus:ring-1 focus:ring-accent-blue"
        />
      </div>
      <button className="flex items-center gap-1.5 px-3 py-1.5 text-xs rounded-md border border-gray-200 dark:border-neutral-700 text-gray-700 dark:text-neutral-200 hover:bg-gray-50 dark:hover:bg-neutral-700">
        Sort by Name
        <ChevronRight size={12} />
      </button>
      <button
        title="Refresh"
        className="p-1.5 rounded-md text-gray-700 dark:text-neutral-200 hover:bg-gray-100 dark:hover:bg-neutral-700"
      >
        <RefreshCcw size={14} />
      </button>
      <button
        className="ml-1 p-1.5 rounded-md bg-accent-blue text-white hover:bg-accent-blue/90"
        title="AI assistant"
      >
        <Sparkles size={14} />
      </button>
    </div>
  );
}

function HoverTooltip({
  label,
  children,
}: {
  label: string;
  children: ReactNode;
}) {
  const [hover, setHover] = useState(false);
  return (
    <div
      className="relative inline-flex"
      onMouseEnter={() => setHover(true)}
      onMouseLeave={() => setHover(false)}
    >
      {children}
      <div
        className={[
          "pointer-events-none absolute left-1/2 top-full mt-1 -translate-x-1/2",
          "px-2 py-1 rounded text-[10px] whitespace-nowrap",
          "bg-neutral-900 text-white shadow-lg z-50",
          "transition-opacity duration-150",
          hover ? "opacity-100" : "opacity-0",
        ].join(" ")}
        role="tooltip"
      >
        {label}
      </div>
    </div>
  );
}