import { useState, useEffect } from "react";
import { useStore } from "../state/store";

export function Settings() {
  const theme = useStore((s) => s.theme);
  const setTheme = useStore((s) => s.setTheme);
  const logLevel = useStore((s) => s.logLevel);
  const setLogLevel = useStore((s) => s.setLogLevel);
  const addr = useStore((s) => s.addr);
  const setAddr = useStore((s) => s.setAddr);

  const [inputAddr, setInputAddr] = useState(addr);
  const [exportMsg, setExportMsg] = useState<string | null>(null);

  useEffect(() => {
    setInputAddr(addr);
  }, [addr]);

  const handleSave = () => {
    const trimmed = inputAddr.trim();
    if (!trimmed) return;
    setAddr(trimmed);
    setExportMsg(`Saved ${trimmed} to localStorage & reconnected`);
  };

  return (
    <div className="space-y-6">
      <h1 className="text-xl font-semibold">Settings</h1>

      <Card title="Application">
        <Row label="Name">bee-gui</Row>
        <Row label="Version">0.1.0 (Tauri)</Row>
        <Row label="Frontend">React 18 + Vite + TypeScript + Tailwind</Row>
      </Card>

      <Card title="Connection">
        <div className="flex items-center gap-2 py-1">
          <label className="w-32 text-xs text-gray-600 dark:text-neutral-400">
            AdminServer:
          </label>
          <input
            value={inputAddr}
            onChange={(e) => setInputAddr(e.target.value)}
            onKeyDown={(e) => {
              if (e.key === "Enter") handleSave();
            }}
            placeholder="127.0.0.1:10001"
            className="flex-1 px-3 py-1.5 text-xs rounded-md border border-gray-200 dark:border-neutral-700 bg-white dark:bg-neutral-900"
          />
          <button
            onClick={handleSave}
            className="px-3 py-1.5 text-xs rounded-md bg-accent-blue text-white hover:bg-accent-blue/90"
          >
            Save
          </button>
        </div>
        {exportMsg && (
          <div className="mt-2 text-xs text-accent-green">{exportMsg}</div>
        )}
      </Card>

      <Card title="Theme">
        <div className="flex items-center gap-2 py-1">
          <label className="w-32 text-xs text-gray-600 dark:text-neutral-400">
            Theme:
          </label>
          <select
            value={theme}
            onChange={(e) =>
              setTheme(e.target.value as "light" | "dark")
            }
            className="px-3 py-1.5 text-xs rounded-md border border-gray-200 dark:border-neutral-700 bg-white dark:bg-neutral-900"
          >
            <option value="light">Light</option>
            <option value="dark">Dark</option>
          </select>
        </div>
      </Card>

      <Card title="Log Level">
        <div className="flex items-center gap-2 py-1">
          <label className="w-32 text-xs text-gray-600 dark:text-neutral-400">
            Log level:
          </label>
          <select
            value={logLevel}
            onChange={(e) =>
              setLogLevel(
                e.target.value as "debug" | "info" | "warn" | "error",
              )
            }
            className="px-3 py-1.5 text-xs rounded-md border border-gray-200 dark:border-neutral-700 bg-white dark:bg-neutral-900"
          >
            <option value="debug">Debug</option>
            <option value="info">Info</option>
            <option value="warn">Warn</option>
            <option value="error">Error</option>
          </select>
          <span className="text-xs text-gray-500 dark:text-neutral-400">
            (applies on next app launch)
          </span>
        </div>
      </Card>

      <Card title="About">
        <p className="text-xs text-gray-600 dark:text-neutral-300">
          Bee cluster management GUI. Backend: Rust + Tauri 2.x. Frontend:
          React + Vite + Tailwind.
        </p>
        <p className="mt-2 text-xs text-gray-500 dark:text-neutral-400">
          Spec: docs/superpowers/specs/2026-07-28-s-tauri-gui-design.md
        </p>
      </Card>
    </div>
  );
}

function Card({ title, children }: { title: string; children: React.ReactNode }) {
  return (
    <section className="bg-white dark:bg-neutral-800 rounded-lg border border-gray-200 dark:border-neutral-700">
      <h2 className="px-4 py-3 text-sm font-medium border-b border-gray-200 dark:border-neutral-700">
        {title}
      </h2>
      <div className="p-4 space-y-2">{children}</div>
    </section>
  );
}

function Row({ label, children }: { label: string; children: React.ReactNode }) {
  return (
    <div className="flex items-center gap-2 py-1">
      <span className="w-32 text-xs text-gray-600 dark:text-neutral-400">
        {label}:
      </span>
      <span className="text-xs">{children}</span>
    </div>
  );
}