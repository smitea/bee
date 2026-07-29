import { useEffect, useState } from "react";

import { useConnection } from "../state/connectionStore";
import { setAddr, testConnection, settingsGet, settingsPut } from "../ipc";

interface Props {
  open: boolean;
  onClose(): void;
}

const DEBOUNCE_MS = 400;

export function SettingsModal({ open, onClose }: Props) {
  const addr = useConnection((s) => s.addr);
  const setStoreAddr = useConnection((s) => s.setAddr);
  const [draft, setDraft] = useState(addr);
  const [saveState, setSaveState] = useState<"idle" | "Saving" | "Saved" | "Error">("idle");
  const [testState, setTestState] = useState<string>("");
  const [initial, setInitial] = useState(addr);

  useEffect(() => {
    if (!open) return;
    let cancelled = false;
    void (async () => {
      const stored = await settingsGet("addr");
      if (!cancelled && stored !== null) {
        setDraft(stored);
        setInitial(stored);
      }
    })();
    return () => {
      cancelled = true;
    };
  }, [open]);

  useEffect(() => {
    if (!open) return;
    if (draft === initial) return;
    setSaveState("Saving");
    const t = setTimeout(async () => {
      try {
        await settingsPut("addr", draft);
        setSaveState("Saved");
        setInitial(draft);
      } catch {
        setSaveState("Error");
      }
    }, DEBOUNCE_MS);
    return () => clearTimeout(t);
  }, [draft, open, initial]);

  const onTest = async () => {
    setTestState("Testing…");
    try {
      const view = await testConnection(draft);
      setTestState(view.status.kind === "Connected" ? "Connected" : `${view.status.kind}`);
    } catch (e) {
      setTestState(`Error: ${(e as Error).message}`);
    }
  };

  const onConnect = async () => {
    try {
      await setAddr(draft);
      setStoreAddr(draft);
      onClose();
    } catch (e) {
      setTestState(`Error: ${(e as Error).message}`);
    }
  };

  if (!open) return null;
  return (
    <div
      className="fixed inset-0 z-50 flex items-center justify-center bg-black/40"
      role="dialog"
      aria-modal="true"
      aria-label="Settings"
    >
      <div className="bg-white dark:bg-neutral-800 rounded-lg shadow-xl w-[640px] max-w-[95vw] h-[480px] flex">
        <aside className="w-44 border-r border-gray-200 dark:border-neutral-700 p-3 text-xs">
          <h2 className="text-sm font-semibold mb-2">Settings</h2>
          <nav className="space-y-1">
            <Section active label="Connection" />
          </nav>
        </aside>
        <main className="flex-1 p-4 flex flex-col">
          <div className="flex items-center justify-between mb-3">
            <h3 className="text-sm font-medium">Connection</h3>
            <span
              className={
                saveState === "Saved"
                  ? "text-[10px] text-accent-green"
                  : saveState === "Error"
                    ? "text-[10px] text-accent-red"
                    : saveState === "Saving"
                      ? "text-[10px] text-gray-500"
                      : "text-[10px] text-transparent"
              }
              aria-live="polite"
            >
              {saveState === "idle" ? "·" : saveState}
            </span>
          </div>
          <label className="text-xs text-gray-500 dark:text-neutral-400" htmlFor="addr">
            AdminServer address
          </label>
          <input
            id="addr"
            value={draft}
            onChange={(e) => setDraft(e.target.value)}
            placeholder="127.0.0.1:8702"
            className="mt-1 px-2 py-1 text-xs font-mono rounded border border-gray-200 dark:border-neutral-700 bg-white dark:bg-neutral-900"
          />
          {testState && (
            <p className="mt-2 text-[10px] text-gray-500 dark:text-neutral-400">{testState}</p>
          )}
          <div className="mt-auto flex items-center gap-2 justify-end">
            <button
              onClick={onTest}
              className="px-3 py-1 text-xs rounded border border-gray-200 dark:border-neutral-700 hover:bg-gray-50 dark:hover:bg-neutral-700"
            >
              Test Connection
            </button>
            <button
              onClick={onConnect}
              className="px-3 py-1 text-xs rounded bg-accent-blue text-white hover:bg-accent-blue/90"
            >
              Connect
            </button>
            <button
              onClick={onClose}
              className="px-3 py-1 text-xs rounded border border-gray-200 dark:border-neutral-700"
            >
              Close
            </button>
          </div>
        </main>
      </div>
    </div>
  );
}

function Section({ label, active }: { label: string; active: boolean }) {
  return (
    <span
      className={[
        "block px-2 py-1 rounded text-xs",
        active
          ? "bg-accent-green/20 text-accent-green"
          : "text-gray-500 dark:text-neutral-400",
      ].join(" ")}
    >
      {label}
    </span>
  );
}
