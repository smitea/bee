import { useStore, type Tab } from "../state/store";
import { Gauge, Database, Workflow, Settings as Cog, Moon, Sun } from "lucide-react";

const TABS: { id: Tab; label: string; Icon: typeof Gauge }[] = [
  { id: "dashboard", label: "Dashboard", Icon: Gauge },
  { id: "dataSources", label: "Data Sources", Icon: Database },
  { id: "pipelines", label: "Pipelines", Icon: Workflow },
  { id: "settings", label: "Settings", Icon: Cog },
];

export function AppBar() {
  const tab = useStore((s) => s.tab);
  const setTab = useStore((s) => s.setTab);
  const theme = useStore((s) => s.theme);
  const toggleTheme = useStore((s) => s.toggleTheme);

  return (
    <header className="flex items-center gap-2 px-6 h-12 bg-white dark:bg-neutral-800 border-b border-gray-200 dark:border-neutral-700">
      <ConnectionPill />
      <div className="flex-1" />
      <nav className="flex items-center gap-1">
        {TABS.map(({ id, label, Icon }) => {
          const active = tab === id;
          return (
            <button
              key={id}
              onClick={() => setTab(id)}
              title={label}
              className={[
                "flex items-center gap-1.5 px-3 py-1.5 rounded-md text-xs transition-colors",
                active
                  ? "bg-accent-blue text-white"
                  : "text-gray-600 dark:text-neutral-300 hover:bg-gray-100 dark:hover:bg-neutral-700",
              ].join(" ")}
            >
              <Icon size={14} />
              {label}
            </button>
          );
        })}
      </nav>
      <div className="flex-1" />
      <button
        onClick={toggleTheme}
        title={theme === "light" ? "Switch to Dark" : "Switch to Light"}
        className="p-2 rounded-md text-gray-600 dark:text-neutral-300 hover:bg-gray-100 dark:hover:bg-neutral-700"
      >
        {theme === "light" ? <Moon size={16} /> : <Sun size={16} />}
      </button>
    </header>
  );
}

function ConnectionPill() {
  const addr = useStore((s) => s.addr);
  return (
    <div className="flex items-center gap-2 px-3 py-1.5 rounded-md bg-gray-100 dark:bg-neutral-700 text-xs">
      <span className="w-2 h-2 rounded-full bg-accent-red" aria-label="Error" />
      <span className="text-gray-700 dark:text-neutral-200 font-medium">
        {addr}
      </span>
    </div>
  );
}