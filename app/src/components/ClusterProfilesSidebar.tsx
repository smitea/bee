import { useEffect, useState } from "react";
import { useQuery, useQueryClient } from "@tanstack/react-query";
import { Hexagon, Plus, X, Trash2, Plug, Server } from "lucide-react";

import {
  clusterProfileList,
  clusterProfileSave,
  clusterProfileRemove,
  clusterProfileActivate,
  clusterProfileMigrateLegacy,
  type ClusterProfileView,
  type LegacyClusterEntry,
} from "../ipc/clusters";
import { setAddr } from "../ipc/connection";

const LS_KEY = "bee-gui.connections";

export function ClusterProfilesSidebar() {
  const qc = useQueryClient();
  const listQ = useQuery<ClusterProfileView[]>({
    queryKey: ["cluster-profiles"],
    queryFn: () => clusterProfileList(),
  });
  const [adding, setAdding] = useState(false);
  const [draftLabel, setDraftLabel] = useState("");
  const [draftAddr, setDraftAddr] = useState("");
  const [draftTenant, setDraftTenant] = useState("0");
  const [error, setError] = useState<string | null>(null);

  useEffect(() => {
    void migrateLegacy(qc);
  }, [qc]);

  const onAdd = async () => {
    setError(null);
    const label = draftLabel.trim();
    const addr = draftAddr.trim();
    const tenant = Number(draftTenant);
    if (!label) {
      setError("label is required");
      return;
    }
    if (!addr) {
      setError("addr is required");
      return;
    }
    if (!Number.isFinite(tenant) || tenant < 0 || tenant > 65535) {
      setError("tenant must be 0..65535");
      return;
    }
    try {
      await clusterProfileSave(label, addr, tenant);
      await qc.invalidateQueries({ queryKey: ["cluster-profiles"] });
      setDraftLabel("");
      setDraftAddr("");
      setDraftTenant("0");
      setAdding(false);
    } catch (e) {
      setError(String(e));
    }
  };

  const onActivate = async (addr: string) => {
    try {
      const view = await clusterProfileActivate(addr);
      await setAddr(view.addr);
      await qc.invalidateQueries({ queryKey: ["cluster-profiles"] });
    } catch (e) {
      setError(String(e));
    }
  };

  const onRemove = async (addr: string) => {
    if (!confirm(`Remove cluster profile ${addr}?`)) return;
    await clusterProfileRemove(addr);
    await qc.invalidateQueries({ queryKey: ["cluster-profiles"] });
  };

  const all = listQ.data ?? [];

  return (
    <div className="border-t border-gray-200 dark:border-neutral-700 px-2 py-2 space-y-1">
      <div className="flex items-center justify-between px-1 text-[10px] font-semibold uppercase tracking-wider text-gray-500 dark:text-neutral-400">
        <span className="flex items-center gap-1">
          <Server size={10} />
          Clusters ({all.length})
        </span>
        <button
          aria-label="Add cluster profile"
          title="Add cluster profile"
          onClick={() => setAdding((s) => !s)}
          className="p-0.5 rounded text-gray-500 hover:bg-gray-100 dark:hover:bg-neutral-700"
        >
          <Plus size={10} />
        </button>
      </div>

      {adding && (
        <div className="px-1 pb-1 space-y-1">
          <input
            aria-label="Cluster label"
            value={draftLabel}
            onChange={(e) => setDraftLabel(e.target.value)}
            placeholder="Label"
            className="w-full px-2 py-1 text-[11px] rounded border border-gray-200 dark:border-neutral-700 bg-white dark:bg-neutral-900"
          />
          <input
            aria-label="Cluster address"
            value={draftAddr}
            onChange={(e) => setDraftAddr(e.target.value)}
            placeholder="127.0.0.1:9999"
            className="w-full px-2 py-1 text-[11px] rounded border border-gray-200 dark:border-neutral-700 bg-white dark:bg-neutral-900 font-mono"
          />
          <input
            aria-label="Cluster tenant"
            value={draftTenant}
            onChange={(e) => setDraftTenant(e.target.value)}
            placeholder="tenant (0..65535)"
            className="w-full px-2 py-1 text-[11px] rounded border border-gray-200 dark:border-neutral-700 bg-white dark:bg-neutral-900 font-mono"
          />
          <div className="flex items-center justify-end gap-1">
            <button
              onClick={() => {
                setAdding(false);
                setError(null);
              }}
              className="px-2 py-0.5 text-[10px] rounded border border-gray-200 dark:border-neutral-700"
            >
              <X size={9} />
            </button>
            <button
              onClick={() => void onAdd()}
              className="px-2 py-0.5 text-[10px] rounded bg-accent-blue text-white"
            >
              save
            </button>
          </div>
        </div>
      )}

      {error && <div className="px-1 text-[10px] text-accent-red">{error}</div>}

      <div className="space-y-0.5 max-h-48 overflow-y-auto">
        {all.length === 0 && !adding && (
          <p className="px-1 py-1 text-[10px] text-gray-400">no saved clusters</p>
        )}
        {all.map((p) => (
          <div
            key={p.id}
            className="group flex items-center gap-1 px-1 py-0.5 rounded text-[11px] hover:bg-gray-100 dark:hover:bg-neutral-700"
          >
            <Hexagon size={9} className="text-gray-400" />
            <button
              onClick={() => void onActivate(p.addr)}
              title={`Connect to ${p.addr}`}
              className="flex-1 text-left truncate"
            >
              <span className="block truncate">{p.label}</span>
              <span className="block font-mono text-[9px] text-gray-400 truncate">
                {p.addr} · t{p.tenant}
              </span>
            </button>
            <button
              aria-label={`Remove ${p.label}`}
              title="Remove"
              onClick={() => void onRemove(p.addr)}
              className="opacity-0 group-hover:opacity-100 p-0.5 text-gray-400 hover:text-accent-red"
            >
              <Trash2 size={9} />
            </button>
            <button
              aria-label={`Activate ${p.label}`}
              title="Activate"
              onClick={() => void onActivate(p.addr)}
              className="opacity-0 group-hover:opacity-100 p-0.5 text-gray-400 hover:text-accent-blue"
            >
              <Plug size={9} />
            </button>
          </div>
        ))}
      </div>
    </div>
  );
}

async function migrateLegacy(qc: ReturnType<typeof useQueryClient>): Promise<void> {
  if (typeof window === "undefined" || typeof localStorage === "undefined") return;
  const raw = localStorage.getItem(LS_KEY);
  if (!raw) return;
  let parsed: unknown;
  try {
    parsed = JSON.parse(raw);
  } catch {
    localStorage.removeItem(LS_KEY);
    return;
  }
  if (!Array.isArray(parsed)) {
    localStorage.removeItem(LS_KEY);
    return;
  }
  const entries: LegacyClusterEntry[] = [];
  for (const item of parsed) {
    if (item && typeof item === "object" && "label" in item && "addr" in item) {
      const obj = item as { label?: unknown; addr?: unknown; tenant?: unknown };
      if (typeof obj.label === "string" && typeof obj.addr === "string") {
        entries.push({
          label: obj.label,
          addr: obj.addr,
          tenant:
            typeof obj.tenant === "number" && obj.tenant >= 0 && obj.tenant <= 65535
              ? obj.tenant
              : null,
        });
      }
    }
  }
  if (entries.length === 0) {
    localStorage.removeItem(LS_KEY);
    return;
  }
  try {
    await clusterProfileMigrateLegacy(entries);
    await qc.invalidateQueries({ queryKey: ["cluster-profiles"] });
  } finally {
    localStorage.removeItem(LS_KEY);
  }
}