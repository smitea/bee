import { useEffect, useState } from "react";
import { Moon, Sun, RefreshCw } from "lucide-react";

import { useUi } from "../state/store";
import { ActivityBar } from "./ActivityBar";
import { ConnectionStatus } from "./ConnectionStatus";
import { useConnection } from "../state/connectionStore";

export const RECONNECT_INTERVAL_MS = 4000;

export function ReconnectButton() {
  const status = useConnection((s) => s.status);
  const addr = useConnection((s) => s.addr);
  const refresh = useConnection((s) => s.refresh);
  const [retrying, setRetrying] = useState(false);
  const [pollId, setPollId] = useState<number | null>(null);

  useEffect(() => {
    if (status.kind === "Connected" && pollId !== null) {
      window.clearInterval(pollId);
      setPollId(null);
      setRetrying(false);
    }
  }, [status.kind, pollId]);

  useEffect(() => {
    return () => {
      if (pollId !== null) window.clearInterval(pollId);
    };
  }, [pollId]);

  if (status.kind !== "Error" && status.kind !== "Disconnected") {
    return null;
  }

  const onClick = async () => {
    setRetrying(true);
    await refresh(addr);
    const id = window.setInterval(() => {
      void refresh(addr);
    }, RECONNECT_INTERVAL_MS);
    setPollId(id);
  };

  return (
    <button
      type="button"
      onClick={onClick}
      disabled={retrying}
      className="inline-flex items-center gap-1 text-[10px] text-accent-blue hover:underline disabled:opacity-50 disabled:cursor-not-allowed"
      data-testid="statusbar-reconnect"
      title="Reconnect to cluster"
    >
      <RefreshCw size={10} className={retrying ? "animate-spin" : undefined} />
      {retrying ? "Reconnecting…" : "Reconnect"}
    </button>
  );
}

export function StatusBar() {
  const theme = useUi((s) => s.theme);
  const toggleTheme = useUi((s) => s.toggleTheme);
  const status = useConnection((s) => s.status);

  return (
    <footer className="flex items-center gap-4 px-3 h-7 text-[10px] text-gray-500 dark:text-neutral-400 bg-white dark:bg-neutral-800 border-t border-gray-200 dark:border-neutral-700">
      <span className="font-medium text-gray-700 dark:text-neutral-200">Bee Client v0.1.0</span>
      <ConnectionStatus onOpenSettings={() => useUi.getState().openSettings()} />
      <ReconnectButton />
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