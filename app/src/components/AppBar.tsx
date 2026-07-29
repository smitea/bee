import { useState, type ReactNode } from "react";
import { useStore, type Tab } from "../state/store";
import { tooltipLabelForTab } from "../tooltip";
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
        {TABS.map(({ id, label, Icon }) => (
          <HoverTooltip key={id} label={tooltipLabelForTab(id)}>
            <button
              onClick={() => setTab(id)}
              className={[
                "flex items-center gap-1.5 px-3 py-1.5 rounded-md text-xs transition-colors",
                tab === id
                  ? "bg-accent-blue text-white"
                  : "text-gray-600 dark:text-neutral-300 hover:bg-gray-100 dark:hover:bg-neutral-700",
              ].join(" ")}
            >
              <Icon size={14} />
              {label}
            </button>
          </HoverTooltip>
        ))}
      </nav>
      <div className="flex-1" />
      <HoverTooltip label={theme === "light" ? "Switch to Dark" : "Switch to Light"}>
        <button
          onClick={toggleTheme}
          className="p-2 rounded-md text-gray-600 dark:text-neutral-300 hover:bg-gray-100 dark:hover:bg-neutral-700"
        >
          {theme === "light" ? <Moon size={16} /> : <Sun size={16} />}
        </button>
      </HoverTooltip>
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

/**
 * Self-drawn tooltip (S-1c follow-up pattern: a styled label that
 * appears on hover after a short delay, themable). Browser-native
 * `title` attribute is the fallback for keyboard users.
 */
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
          "px-2 py-1 rounded text-[10px] whitespace-nowrap pointer-events-none",
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