import { Moon, Sun } from "lucide-react";
import { useUi } from "../state/store";
import { ActivityBar } from "./ActivityBar";
import { ConnectionStatus } from "./ConnectionStatus";
import { useConnection } from "../state/connectionStore";

export function StatusBar() {
  const theme = useUi((s) => s.theme);
  const toggleTheme = useUi((s) => s.toggleTheme);
  const status = useConnection((s) => s.status);

  return (
    <footer className="flex items-center gap-4 px-3 h-7 text-[10px] text-gray-500 dark:text-neutral-400 bg-white dark:bg-neutral-800 border-t border-gray-200 dark:border-neutral-700">
      <span className="font-medium text-gray-700 dark:text-neutral-200">Bee Client v0.1.0</span>
      <ConnectionStatus onOpenSettings={() => useUi.getState().openSettings()} />
      <ActivityBar />
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