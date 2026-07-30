import { useEffect, useState } from "react";
import { useQuery, useQueryClient } from "@tanstack/react-query";
import { Plus, Trash2, Plug, X, Server } from "lucide-react";

import { useConnection } from "../state/connectionStore";
import { useTenant } from "../state/tenantStore";
import {
  setAddr,
  testConnection,
  settingsGet,
  settingsPut,
  tenantGet,
  tenantSet,
} from "../ipc";
import {
  clusterProfileList,
  clusterProfileSave,
  clusterProfileRemove,
  clusterProfileActivate,
  type ClusterProfileView,
} from "../ipc/clusters";
import { ClusterStatusDot } from "./ClusterStatusDot";

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
  { id: "connection", label: "Connection", description: "Saved Bee clusters, AdminServer address, and reachability." },
  { id: "tenant", label: "Tenant", description: "Active tenant for new Applications and Datasources." },
  { id: "appearance", label: "Appearance", description: "Theme, density, and visual preferences." },
  { id: "logging", label: "Logging", description: "Log verbosity and routing." },
  { id: "diagnostics", label: "Diagnostics", description: "Diagnostic export and troubleshooting." },
  { id: "raft", label: "Raft", description: "Raft tunables and quorum behaviour." },
  { id: "kv", label: "KV", description: "KV Cluster storage options." },
  { id: "scheduling", label: "Scheduling", description: "Work-Stealing and rebalancing." },
  { id: "plugins", label: "Plugins", description: "Plugin registry and Adapter schemas." },
  { id: "security", label: "Security", description: "Authentication and redaction rules." },
];

export function SettingsModal({ open, onClose }: Props) {
  const [section, setSection] = useState<SectionId>("connection");

  const addr = useConnection((s) => s.addr);
  const setStoreAddr = useConnection((s) => s.setAddr);
  const [draft, setDraft] = useState(addr);
  const [saveState, setSaveState] = useState<"idle" | "Saving" | "Saved" | "Error">("idle");
  const [testState, setTestState] = useState<string>("");
  const [initial, setInitial] = useState(addr);

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
        setDraft(stored);
        setInitial(stored);
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
              draft={draft}
              setDraft={setDraft}
              saveState={saveState}
              testState={testState}
              onTest={onTest}
              onConnect={onConnect}
              onSwitchCluster={async (a) => {
                setDraft(a);
                try {
                  await clusterProfileActivate(a);
                  await setAddr(a);
                  setStoreAddr(a);
                  setInitial(a);
                } catch {
                  /* surfaced via store */
                }
              }}
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

          {!["connection", "tenant"].includes(section) && (
            <PlaceholderSection
              title={SECTIONS.find((s) => s.id === section)?.label ?? ""}
              description={SECTIONS.find((s) => s.id === section)?.description ?? ""}
            />
          )}

          <div className="mt-auto flex items-center gap-2 justify-end">
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
  draft,
  setDraft,
  saveState,
  testState,
  onTest,
  onConnect,
  onSwitchCluster,
}: {
  addr: string;
  draft: string;
  setDraft: (v: string) => void;
  saveState: "idle" | "Saving" | "Saved" | "Error";
  testState: string;
  onTest(): void | Promise<void>;
  onConnect(): void | Promise<void>;
  onSwitchCluster(addr: string): void | Promise<void>;
}) {
  const qc = useQueryClient();
  const status = useConnection((s) => s.status);
  const listQ = useQuery<ClusterProfileView[]>({
    queryKey: ["cluster-profiles"],
    queryFn: () => clusterProfileList(),
  });
  const [adding, setAdding] = useState(false);
  const [editingId, setEditingId] = useState<number | null>(null);
  const [draftLabel, setDraftLabel] = useState("");
  const [draftAddr, setDraftAddr] = useState("");
  const [draftTenant, setDraftTenant] = useState("0");
  const [error, setError] = useState<string | null>(null);

  const all = listQ.data ?? [];

  const beginAdd = () => {
    setEditingId(null);
    setDraftLabel("");
    setDraftAddr("");
    setDraftTenant("0");
    setError(null);
    setAdding(true);
  };

  const beginEdit = (p: ClusterProfileView) => {
    setAdding(false);
    setEditingId(p.id);
    setDraftLabel(p.label);
    setDraftAddr(p.addr);
    setDraftTenant(String(p.tenant));
    setError(null);
  };

  const cancel = () => {
    setAdding(false);
    setEditingId(null);
    setError(null);
  };

  const onSave = async () => {
    setError(null);
    const label = draftLabel.trim();
    const addrV = draftAddr.trim();
    const tenant = Number(draftTenant);
    if (!label) {
      setError("label is required");
      return;
    }
    if (!addrV) {
      setError("addr is required");
      return;
    }
    if (!Number.isFinite(tenant) || tenant < 0 || tenant > 65535) {
      setError("tenant must be 0..65535");
      return;
    }
    try {
      if (adding) {
        await clusterProfileSave(label, addrV, tenant);
      } else if (editingId !== null) {
        await clusterProfileRemove(all.find((p) => p.id === editingId)?.addr ?? "");
        await clusterProfileSave(label, addrV, tenant);
      }
      await qc.invalidateQueries({ queryKey: ["cluster-profiles"] });
      cancel();
    } catch (e) {
      setError(String(e));
    }
  };

  const onRemove = async (p: ClusterProfileView) => {
    try {
      await clusterProfileRemove(p.addr);
      await qc.invalidateQueries({ queryKey: ["cluster-profiles"] });
    } catch (e) {
      setError(String(e));
    }
  };

  return (
    <section>
      <div className="flex items-center justify-between mb-3">
        <h3 className="text-sm font-medium">Connection</h3>
        <button
          type="button"
          onClick={beginAdd}
          className="flex items-center gap-1 px-2 py-1 text-[11px] rounded border border-gray-200 dark:border-neutral-700 hover:bg-gray-50 dark:hover:bg-neutral-700"
          aria-label="Add cluster"
        >
          <Plus size={10} />
          Add cluster
        </button>
      </div>
      <p className="text-[10px] text-gray-500 dark:text-neutral-400 mb-2">
        Saved Bee clusters. Selecting a row pre-fills the address below.
      </p>

      {(adding || editingId !== null) && (
        <div className="space-y-2 p-2 rounded border border-gray-200 dark:border-neutral-700 mb-2">
          <input
            aria-label="Cluster label"
            value={draftLabel}
            onChange={(e) => setDraftLabel(e.target.value)}
            placeholder="Label"
            className="w-full px-2 py-1 text-xs rounded border border-gray-200 dark:border-neutral-700 bg-white dark:bg-neutral-900"
          />
          <input
            aria-label="Cluster address"
            value={draftAddr}
            onChange={(e) => setDraftAddr(e.target.value)}
            placeholder="127.0.0.1:9999"
            className="w-full px-2 py-1 text-xs font-mono rounded border border-gray-200 dark:border-neutral-700 bg-white dark:bg-neutral-900"
          />
          <input
            aria-label="Cluster tenant"
            value={draftTenant}
            onChange={(e) => setDraftTenant(e.target.value)}
            placeholder="tenant (0..65535)"
            className="w-full px-2 py-1 text-xs font-mono rounded border border-gray-200 dark:border-neutral-700 bg-white dark:bg-neutral-900"
          />
          {error && <p className="text-[10px] text-accent-red">{error}</p>}
          <div className="flex items-center justify-end gap-1">
            <button
              type="button"
              onClick={cancel}
              className="px-2 py-0.5 text-[11px] rounded border border-gray-200 dark:border-neutral-700"
            >
              <X size={10} />
            </button>
            <button
              type="button"
              onClick={() => void onSave()}
              className="px-2 py-0.5 text-[11px] rounded bg-accent-blue text-white"
            >
              save
            </button>
          </div>
        </div>
      )}

      {all.length === 0 ? (
        <p className="text-[11px] text-gray-400 px-2 py-2" data-testid="clusters-list">
          no saved clusters
        </p>
      ) : (
        <ul className="space-y-1 mb-3" data-testid="clusters-list">
          {all.map((p) => {
            const isActive = p.addr.trim() === addr.trim();
            return (
              <li
                key={p.id}
                className="flex items-center gap-2 px-2 py-1.5 rounded border border-gray-200 dark:border-neutral-700"
              >
                <ClusterStatusDot
                  profileAddr={p.addr}
                  activeAddr={addr}
                  status={isActive ? status : { kind: "Disconnected" }}
                />
                <button
                  type="button"
                  onClick={() => {
                    setDraft(p.addr);
                    void onSwitchCluster(p.addr);
                  }}
                  className="flex-1 min-w-0 text-left"
                  aria-label={`Select ${p.label}`}
                  title={`Switch address to ${p.addr}`}
                >
                  <div className="text-xs truncate">{p.label}</div>
                  <div className="font-mono text-[10px] text-gray-400 truncate">
                    {p.addr} · t{p.tenant}
                  </div>
                </button>
                <button
                  type="button"
                  aria-label={`Connect ${p.label}`}
                  title="Connect"
                  onClick={() => void onSwitchCluster(p.addr)}
                  className="p-1 rounded text-gray-400 hover:text-accent-blue"
                >
                  <Plug size={11} />
                </button>
                <button
                  type="button"
                  aria-label={`Edit ${p.label}`}
                  title="Edit"
                  onClick={() => beginEdit(p)}
                  className="p-1 rounded text-gray-400 hover:text-accent-blue"
                >
                  <Server size={11} />
                </button>
                <button
                  type="button"
                  aria-label={`Remove ${p.label}`}
                  title="Remove"
                  onClick={() => void onRemove(p)}
                  className="p-1 rounded text-gray-400 hover:text-accent-red"
                >
                  <Trash2 size={11} />
                </button>
              </li>
            );
          })}
        </ul>
      )}

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
      <label className="text-xs text-gray-500 dark:text-neutral-400" htmlFor="addr">
        AdminServer address
      </label>
      <input
        id="addr"
        value={draft}
        onChange={(e) => setDraft(e.target.value)}
        placeholder="127.0.0.1:8702"
        className="mt-1 w-full px-2 py-1 text-xs font-mono rounded border border-gray-200 dark:border-neutral-700 bg-white dark:bg-neutral-900"
      />
      {testState && (
        <p className="mt-2 text-[10px] text-gray-500 dark:text-neutral-400">{testState}</p>
      )}
      <div className="mt-3 flex items-center gap-2 justify-end">
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
      </div>
    </section>
  );
}
