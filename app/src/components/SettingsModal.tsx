import { useEffect, useState } from "react";
import { useQuery, useQueryClient } from "@tanstack/react-query";
import { Plug, Power, PowerOff, RefreshCw } from "lucide-react";

import { useConnection } from "../state/connectionStore";
import { useTenant } from "../state/tenantStore";
import {
  setAddr,
  testConnection,
  settingsGet,
  settingsPut,
  tenantGet,
  tenantSet,
  pluginList,
  pluginScanDirectory,
  pluginDefaultDir,
  pluginLastDir,
  pluginSettingsGet,
  pluginSettingsSet,
  type PluginSummary,
  type PluginSettingView,
} from "../ipc";
import { useNavigation } from "../state/navigationStore";

interface Props {
  open: boolean;
  onClose(): void;
}

const DEBOUNCE_MS = 400;

type SectionId =
  | "client"
  | "connection"
  | "tenant"
  | "appearance"
  | "logging"
  | "diagnostics"
  | "raft"
  | "kv"
  | "scheduling"
  | "plugins"
  | "security";

const SECTIONS: { id: SectionId; label: string; description: string }[] = [
  { id: "client", label: "Client", description: "Bee Client desktop preferences." },
  { id: "connection", label: "Connection", description: "Active Bee AdminServer address and reachability." },
  { id: "tenant", label: "Tenant", description: "Active tenant for new Applications and Datasources." },
  { id: "appearance", label: "Appearance", description: "Theme, density, and visual preferences." },
  { id: "logging", label: "Logging", description: "Log verbosity and routing." },
  { id: "diagnostics", label: "Diagnostics", description: "Diagnostic export and troubleshooting." },
  { id: "raft", label: "Raft", description: "Raft tunables and quorum behaviour." },
  { id: "kv", label: "KV", description: "KV Cluster storage options." },
  { id: "scheduling", label: "Scheduling", description: "Work-Stealing and rebalancing." },
  { id: "plugins", label: "Plugins", description: "Loaded plugins, enable/disable, configuration." },
  { id: "security", label: "Security", description: "Authentication and redaction rules." },
];

export function SettingsModal({ open, onClose }: Props) {
  const [section, setSection] = useState<SectionId>("connection");
  const cancelAction = useNavigationStoreBack(onClose);

  const addr = useConnection((s) => s.addr);
  const setStoreAddr = useConnection((s) => s.setAddr);
  const [draftLabel, setDraftLabel] = useState("default");
  const [draftAddr, setDraftAddr] = useState(addr);
  const [initialAddr, setInitialAddr] = useState(addr);
  const [saveState, setSaveState] = useState<"idle" | "Saving" | "Saved" | "Error">("idle");
  const [testState, setTestState] = useState<string>("");

  const [tenantDraft, setTenantDraft] = useState<string>("0");
  const [tenantInitial, setTenantInitial] = useState<string>("0");
  const [tenantSaveState, setTenantSaveState] = useState<"idle" | "Saving" | "Saved" | "Error">("idle");
  const [tenantError, setTenantError] = useState<string>("");
  const setStoreTenant = useTenant((s) => s.set);
  const tenantHydrated = useTenant((s) => s.hydrated);
  const refreshTenant = useTenant((s) => s.refresh);

  useEffect(() => {
    if (!open) return;
    let cancelled = false;
    void (async () => {
      const stored = await settingsGet("addr");
      if (!cancelled && stored !== null) {
        setDraftAddr(stored);
        setInitialAddr(stored);
      }
      const t = await tenantGet();
      if (!cancelled) {
        setTenantDraft(String(t));
        setTenantInitial(String(t));
      }
    })();
    return () => {
      cancelled = true;
    };
  }, [open]);

  useEffect(() => {
    if (!open) return;
    if (draftAddr === initialAddr) return;
    setSaveState("Saving");
    const t = setTimeout(async () => {
      try {
        await settingsPut("addr", draftAddr);
        setSaveState("Saved");
        setInitialAddr(draftAddr);
      } catch {
        setSaveState("Error");
      }
    }, DEBOUNCE_MS);
    return () => clearTimeout(t);
  }, [draftAddr, open, initialAddr]);

  useEffect(() => {
    if (!open) return;
    if (tenantDraft === tenantInitial) return;
    setTenantSaveState("Saving");
    setTenantError("");
    const parsed = Number(tenantDraft);
    if (!Number.isFinite(parsed) || parsed < 0 || parsed > 65535) {
      setTenantSaveState("Error");
      setTenantError("tenant must be a number between 0 and 65535");
      return;
    }
    const t = setTimeout(async () => {
      try {
        await tenantSet(parsed);
        await setStoreTenant(parsed);
        setTenantSaveState("Saved");
        setTenantInitial(tenantDraft);
      } catch (e) {
        setTenantSaveState("Error");
        setTenantError(String(e));
      }
    }, DEBOUNCE_MS);
    return () => clearTimeout(t);
  }, [tenantDraft, open, tenantInitial, setStoreTenant]);

  const onTest = async () => {
    setTestState("Testing…");
    try {
      const view = await testConnection(draftAddr);
      setTestState(view.status.kind === "Connected" ? "Connected" : `${view.status.kind}`);
    } catch (e) {
      setTestState(`Error: ${(e as Error).message}`);
    }
  };

  const onConnect = async () => {
    try {
      await setAddr(draftAddr);
      setStoreAddr(draftAddr);
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
      <div className="bg-white dark:bg-neutral-800 rounded-lg shadow-xl w-[640px] max-w-[95vw] h-[520px] flex">
        <aside className="w-44 border-r border-gray-200 dark:border-neutral-700 p-3 text-xs overflow-y-auto">
          <h2 className="text-sm font-semibold mb-2">Settings</h2>
          <nav className="space-y-1">
            {SECTIONS.map((s) => (
              <Section
                key={s.id}
                label={s.label}
                active={section === s.id}
                onClick={() => setSection(s.id)}
              />
            ))}
          </nav>
        </aside>
        <main className="flex-1 p-4 flex flex-col gap-4 overflow-y-auto">
          {section === "connection" && (
            <ConnectionSection
              addr={addr}
              draftLabel={draftLabel}
              draftAddr={draftAddr}
              setDraftLabel={setDraftLabel}
              setDraftAddr={setDraftAddr}
              saveState={saveState}
              testState={testState}
              onTest={onTest}
              onConnect={onConnect}
            />
          )}

          {section === "tenant" && (
            <section>
              <div className="flex items-center justify-between mb-3">
                <h3 className="text-sm font-medium">Tenant</h3>
                <span
                  className={
                    tenantSaveState === "Saved"
                      ? "text-[10px] text-accent-green"
                      : tenantSaveState === "Error"
                        ? "text-[10px] text-accent-red"
                        : tenantSaveState === "Saving"
                          ? "text-[10px] text-gray-500"
                          : "text-[10px] text-transparent"
                  }
                  aria-live="polite"
                >
                  {tenantSaveState === "idle" ? "·" : tenantSaveState}
                </span>
              </div>
              <p className="text-[10px] text-gray-500 dark:text-neutral-400 mb-2">
                Active tenant for new Applications and Datasources.
              </p>
              <label className="text-xs text-gray-500 dark:text-neutral-400" htmlFor="tenant">
                Tenant (0..65535)
              </label>
              <input
                id="tenant"
                aria-label="Active tenant"
                value={tenantDraft}
                onChange={(e) => setTenantDraft(e.target.value)}
                onFocus={() => {
                  if (!tenantHydrated) void refreshTenant();
                }}
                className="mt-1 w-full px-2 py-1 text-xs font-mono rounded border border-gray-200 dark:border-neutral-700 bg-white dark:bg-neutral-900"
              />
              {tenantError && (
                <p className="mt-2 text-[10px] text-accent-red">{tenantError}</p>
              )}
            </section>
          )}

          {section === "plugins" && <PluginsSection />}

          {!["connection", "tenant", "plugins"].includes(section) && (
            <PlaceholderSection
              title={SECTIONS.find((s) => s.id === section)?.label ?? ""}
              description={SECTIONS.find((s) => s.id === section)?.description ?? ""}
            />
          )}

          <div className="mt-auto flex items-center gap-2 justify-end">
            <button
              onClick={cancelAction}
              className="px-3 py-1 text-xs rounded border border-gray-200 dark:border-neutral-700"
            >
              Cancel
            </button>
          </div>
        </main>
      </div>
    </div>
  );
}

function useNavigationStoreBack(fallback: () => void): () => void {
  const { back, canGoBack } = useNavigation();
  return () => {
    if (canGoBack) {
      back();
    } else {
      fallback();
    }
  };
}

function Section({
  label,
  active,
  onClick,
}: {
  label: string;
  active?: boolean;
  onClick?(): void;
}) {
  return (
    <button
      type="button"
      onClick={onClick}
      className={[
        "block w-full text-left px-2 py-1 rounded text-xs",
        active
          ? "bg-accent-green/20 text-accent-green"
          : "text-gray-500 dark:text-neutral-400 hover:bg-gray-100 dark:hover:bg-neutral-700",
      ].join(" ")}
    >
      {label}
    </button>
  );
}

function PlaceholderSection({ title, description }: { title: string; description: string }) {
  return (
    <section>
      <h3 className="text-sm font-medium mb-2">{title}</h3>
      <p className="text-[10px] text-gray-500 dark:text-neutral-400 mb-2">{description}</p>
      <div className="rounded border border-dashed border-gray-200 dark:border-neutral-700 p-4 text-[11px] text-gray-400">
        Settings for this category land in a later slice.
      </div>
    </section>
  );
}

function ConnectionSection({
  addr,
  draftLabel,
  draftAddr,
  setDraftLabel,
  setDraftAddr,
  saveState,
  testState,
  onTest,
  onConnect,
}: {
  addr: string;
  draftLabel: string;
  draftAddr: string;
  setDraftLabel: (v: string) => void;
  setDraftAddr: (v: string) => void;
  saveState: "idle" | "Saving" | "Saved" | "Error";
  testState: string;
  onTest(): void | Promise<void>;
  onConnect(): void | Promise<void>;
}) {
  return (
    <section>
      <div className="flex items-center justify-between mb-3">
        <h3 className="text-sm font-medium">Connection</h3>
      </div>
      <p className="text-[10px] text-gray-500 dark:text-neutral-400 mb-3">
        Connect to a Bee AdminServer. Topology is auto-discovered from the active address.
      </p>

      <label className="text-xs text-gray-500 dark:text-neutral-400" htmlFor="conn-label">
        Label
      </label>
      <input
        id="conn-label"
        aria-label="Connection label"
        value={draftLabel}
        onChange={(e) => setDraftLabel(e.target.value)}
        placeholder="default"
        className="mt-1 mb-3 w-full px-2 py-1 text-xs rounded border border-gray-200 dark:border-neutral-700 bg-white dark:bg-neutral-900"
      />

      <label className="text-xs text-gray-500 dark:text-neutral-400" htmlFor="addr">
        AdminServer address
      </label>
      <input
        id="addr"
        aria-label="AdminServer address"
        value={draftAddr}
        onChange={(e) => setDraftAddr(e.target.value)}
        placeholder="127.0.0.1:8702"
        className="mt-1 w-full px-2 py-1 text-xs font-mono rounded border border-gray-200 dark:border-neutral-700 bg-white dark:bg-neutral-900"
      />
      <div className="mt-1 text-[10px] text-gray-500 dark:text-neutral-400">
        current: <span className="font-mono">{addr}</span>
      </div>

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

      {testState && (
        <p className="mt-2 text-[10px] text-gray-500 dark:text-neutral-400">{testState}</p>
      )}
      <div className="mt-3 flex items-center gap-2 justify-end">
        <button
          onClick={() => void onTest()}
          className="px-3 py-1 text-xs rounded border border-gray-200 dark:border-neutral-700 hover:bg-gray-50 dark:hover:bg-neutral-700"
        >
          Test Connection
        </button>
        <button
          onClick={() => void onConnect()}
          className="flex items-center gap-1 px-3 py-1 text-xs rounded bg-accent-blue text-white hover:bg-accent-blue/90"
        >
          <Plug size={11} />
          Connect
        </button>
      </div>
    </section>
  );
}

function PluginsSection() {
  const queryClient = useQueryClient();
  const pluginsQ = useQuery<PluginSummary[]>({
    queryKey: ["plugins-list"],
    queryFn: () => pluginList(),
  });
  const [pluginDir, setPluginDir] = useState<string>("");
  const [scanState, setScanState] = useState<"idle" | "Scanning" | "Scanned" | "Error">("idle");
  const [scanError, setScanError] = useState<string>("");

  useEffect(() => {
    let cancelled = false;
    void (async () => {
      try {
        const last = await pluginLastDir();
        const fallback = await pluginDefaultDir();
        if (!cancelled) {
          setPluginDir(last && last.trim() !== "" ? last : fallback);
        }
      } catch {
        if (!cancelled) {
          try {
            const fallback = await pluginDefaultDir();
            if (!cancelled) setPluginDir(fallback);
          } catch {
            if (!cancelled) setPluginDir("");
          }
        }
      }
    })();
    return () => {
      cancelled = true;
    };
  }, []);

  const all = pluginsQ.data ?? [];

  const onReload = async () => {
    if (!pluginDir) return;
    setScanState("Scanning");
    setScanError("");
    try {
      await pluginScanDirectory(pluginDir);
      await settingsPut("plugin_dir", pluginDir);
      await queryClient.invalidateQueries({ queryKey: ["plugins-list"] });
      setScanState("Scanned");
    } catch (e) {
      setScanError(String((e as Error).message ?? e));
      setScanState("Error");
    }
  };

  return (
    <section>
      <div className="flex items-center justify-between mb-3">
        <h3 className="text-sm font-medium">Plugins</h3>
        <span
          className="text-[10px] text-gray-500 dark:text-neutral-400"
          data-testid="plugins-count"
        >
          {all.length} loaded
        </span>
      </div>
      <p className="text-[10px] text-gray-500 dark:text-neutral-400 mb-2">
        Plugins loaded via bee_plugin_sdk. Toggle enabled state and edit per-plugin
        configuration (JSON).
      </p>
      <div className="mb-3 rounded border border-gray-200 dark:border-neutral-700 p-2 space-y-2">
        <label
          className="text-[10px] uppercase tracking-wider text-gray-500 dark:text-neutral-400"
          htmlFor="plugin-dir"
        >
          Plugin directory
        </label>
        <input
          id="plugin-dir"
          aria-label="Plugin directory"
          value={pluginDir}
          onChange={(e) => setPluginDir(e.target.value)}
          placeholder="/path/to/plugins"
          className="w-full px-2 py-1 text-xs font-mono rounded border border-gray-200 dark:border-neutral-700 bg-white dark:bg-neutral-900"
          data-testid="plugin-dir-input"
        />
        <div className="flex items-center justify-between">
          <span
            className={
              scanState === "Scanned"
                ? "text-[10px] text-accent-green"
                : scanState === "Error"
                  ? "text-[10px] text-accent-red"
                  : scanState === "Scanning"
                    ? "text-[10px] text-gray-500"
                    : "text-[10px] text-transparent"
            }
            aria-live="polite"
            data-testid="plugin-scan-status"
          >
            {scanState === "idle" ? "·" : scanState}
          </span>
          <button
            type="button"
            onClick={() => void onReload()}
            disabled={scanState === "Scanning" || !pluginDir}
            data-testid="plugin-reload"
            className="flex items-center gap-1 px-2 py-1 text-[10px] rounded bg-accent-blue text-white hover:bg-accent-blue/90 disabled:opacity-50"
          >
            <RefreshCw
              size={11}
              className={scanState === "Scanning" ? "animate-spin" : ""}
            />
            Reload from disk
          </button>
        </div>
        {scanError && (
          <p className="text-[10px] text-accent-red" data-testid="plugin-scan-error">
            {scanError}
          </p>
        )}
      </div>
      {all.length === 0 ? (
        <p className="text-[11px] text-gray-400 px-2 py-2">no plugins loaded</p>
      ) : (
        <ul className="space-y-2">
          {all.map((p) => (
            <PluginRow key={p.id} plugin={p} />
          ))}
        </ul>
      )}
    </section>
  );
}

function PluginRow({ plugin }: { plugin: PluginSummary }) {
  const [setting, setSetting] = useState<PluginSettingView | null>(null);
  const [configDraft, setConfigDraft] = useState<string>("{}");
  const [error, setError] = useState<string | null>(null);
  const [busy, setBusy] = useState(false);

  useEffect(() => {
    let cancelled = false;
    void pluginSettingsGet(plugin.name).then((s) => {
      if (cancelled) return;
      if (s) {
        setSetting(s);
        setConfigDraft(s.config_json || "{}");
      } else {
        setSetting({
          plugin_name: plugin.name,
          enabled: true,
          config_json: "{}",
          updated_at: 0,
        });
        setConfigDraft("{}");
      }
    });
    return () => {
      cancelled = true;
    };
  }, [plugin.name]);

  const enabled = setting?.enabled ?? true;

  const onToggle = async (next: boolean) => {
    setBusy(true);
    setError(null);
    try {
      const updated = await pluginSettingsSet(plugin.name, next, configDraft);
      setSetting(updated);
    } catch (e) {
      setError(String(e));
    } finally {
      setBusy(false);
    }
  };

  const onSaveConfig = async () => {
    setBusy(true);
    setError(null);
    try {
      JSON.parse(configDraft);
    } catch {
      setError("config must be valid JSON");
      setBusy(false);
      return;
    }
    try {
      const updated = await pluginSettingsSet(plugin.name, enabled, configDraft);
      setSetting(updated);
    } catch (e) {
      setError(String(e));
    } finally {
      setBusy(false);
    }
  };

  return (
    <li
      data-testid={`plugin-row-${plugin.name}`}
      className="rounded border border-gray-200 dark:border-neutral-700 p-2 space-y-2"
    >
      <div className="flex items-center gap-2">
        <button
          type="button"
          aria-label={enabled ? "Disable plugin" : "Enable plugin"}
          title={enabled ? "Disable" : "Enable"}
          disabled={busy}
          onClick={() => void onToggle(!enabled)}
          className={[
            "p-1 rounded",
            enabled
              ? "text-accent-green hover:text-accent-green/70"
              : "text-gray-400 hover:text-accent-blue",
          ].join(" ")}
        >
          {enabled ? <Power size={11} /> : <PowerOff size={11} />}
        </button>
        <span className="text-xs font-medium flex-1">{plugin.name}</span>
        <span className="text-[10px] text-gray-500 dark:text-neutral-400 font-mono">
          v{plugin.version}
        </span>
      </div>
      <div className="text-[10px] text-gray-500 dark:text-neutral-400">
        adapters: {plugin.adapters.join(", ") || "—"}
        {plugin.handlers.length > 0 && ` · handlers: ${plugin.handlers.join(", ")}`}
      </div>
      <details className="text-[10px]">
        <summary className="cursor-pointer text-gray-500 dark:text-neutral-400">
          configuration (JSON)
        </summary>
        <textarea
          aria-label={`plugin ${plugin.name} config`}
          value={configDraft}
          onChange={(e) => setConfigDraft(e.target.value)}
          rows={4}
          className="mt-1 w-full px-2 py-1 text-[10px] font-mono rounded border border-gray-200 dark:border-neutral-700 bg-white dark:bg-neutral-900"
        />
        <div className="mt-1 flex items-center gap-2">
          <button
            type="button"
            disabled={busy}
            onClick={() => void onSaveConfig()}
            className="px-2 py-0.5 text-[10px] rounded bg-accent-blue text-white disabled:opacity-50"
          >
            save config
          </button>
          {error && <span className="text-accent-red text-[10px]">{error}</span>}
        </div>
      </details>
    </li>
  );
}
