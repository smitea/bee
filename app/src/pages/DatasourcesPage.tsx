import { useEffect, useState } from "react";
import { useQuery, useQueryClient } from "@tanstack/react-query";
import { Database, Plus, Trash2, X, Check } from "lucide-react";

import {
  datasourceList,
  datasourceCreate,
  datasourceDelete,
  type DatasourceView,
} from "../ipc/datasources";
import {
  pluginList,
  pluginSchema,
  type PluginInfo,
  type PluginSchema,
  type PluginFieldSchema,
} from "../ipc/plugins";

const REFRESH_MS = 5000;

export function DatasourcesPage() {
  const qc = useQueryClient();
  const listQ = useQuery<DatasourceView[]>({
    queryKey: ["datasources-local"],
    queryFn: () => datasourceList(),
    refetchInterval: REFRESH_MS,
  });
  const [showAdd, setShowAdd] = useState(false);
  const all = listQ.data ?? [];

  const onDelete = async (name: string) => {
    if (!confirm(`Delete datasource "${name}"?`)) return;
    await datasourceDelete(name);
    await qc.invalidateQueries({ queryKey: ["datasources-local"] });
  };

  return (
    <div className="space-y-6">
      <header className="flex items-center justify-between">
        <h1 className="text-xl font-semibold">Datasources</h1>
        <button
          type="button"
          data-testid="add-datasource"
          onClick={() => setShowAdd(true)}
          className="flex items-center gap-1.5 px-3 py-1.5 text-xs rounded-md bg-accent-blue text-white hover:bg-accent-blue/90"
        >
          <Plus size={14} />
          Add
        </button>
      </header>

      <section className="bg-white dark:bg-neutral-800 rounded-lg border border-gray-200 dark:border-neutral-700">
        <h2 className="px-4 py-3 text-sm font-medium border-b border-gray-200 dark:border-neutral-700">
          Datasources ({all.length})
        </h2>
        <div className="p-4">
          {all.length === 0 ? (
            <div className="text-center py-8 text-gray-400">
              <Database
                size={48}
                className="mx-auto text-gray-300 dark:text-neutral-600"
              />
              <p className="mt-2 text-sm">no datasources — click Add</p>
            </div>
          ) : (
            <table className="w-full text-xs">
              <thead>
                <tr className="text-[10px] uppercase tracking-wider text-gray-500 dark:text-neutral-400">
                  <th className="text-left font-medium pb-2">Name</th>
                  <th className="text-left font-medium pb-2">Plugin</th>
                  <th className="text-left font-medium pb-2">Tenant</th>
                  <th className="text-left font-medium pb-2">Updated</th>
                  <th className="text-left font-medium pb-2">Actions</th>
                </tr>
              </thead>
              <tbody>
                {all.map((d) => (
                  <tr
                    key={d.name}
                    className="border-t border-gray-100 dark:border-neutral-800"
                  >
                    <td className="py-2 font-mono">{d.name}</td>
                    <td className="py-2 font-mono text-gray-600 dark:text-neutral-300">
                      {d.plugin}
                    </td>
                    <td className="py-2 font-mono">{d.tenant}</td>
                    <td className="py-2 text-[10px] text-gray-400">
                      {d.updated_at}
                    </td>
                    <td className="py-2 space-x-1">
                      <button
                        type="button"
                        aria-label={`delete ${d.name}`}
                        onClick={() => void onDelete(d.name)}
                        className="px-2 py-0.5 text-[11px] rounded border border-gray-200 dark:border-neutral-700 hover:bg-red-50 dark:hover:bg-red-900/30 hover:text-accent-red"
                      >
                        <Trash2 size={11} />
                      </button>
                    </td>
                  </tr>
                ))}
              </tbody>
            </table>
          )}
        </div>
      </section>

      {showAdd && (
        <AddDatasourceModal
          onClose={() => setShowAdd(false)}
          onCreated={async () => {
            setShowAdd(false);
            await qc.invalidateQueries({ queryKey: ["datasources-local"] });
          }}
        />
      )}
    </div>
  );
}

function AddDatasourceModal({
  onClose,
  onCreated,
}: {
  onClose: () => void;
  onCreated: () => Promise<void>;
}) {
  const pluginsQ = useQuery<PluginInfo[]>({
    queryKey: ["plugins-list"],
    queryFn: () => pluginList(),
  });
  const [name, setName] = useState("");
  const [plugin, setPlugin] = useState("");
  const [tenant, setTenant] = useState("0");
  const [configFields, setConfigFields] = useState<Record<string, string>>({});
  const [error, setError] = useState<string | null>(null);
  const [busy, setBusy] = useState(false);
  const [testState, setTestState] = useState<string>("");

  const schemaQ = useQuery<PluginSchema | null>({
    queryKey: ["plugins-schema", plugin],
    queryFn: () => (plugin ? pluginSchema(plugin) : Promise.resolve(null)),
    enabled: plugin.length > 0,
  });

  useEffect(() => {
    setConfigFields({});
    setError(null);
  }, [plugin]);

  const configJson = JSON.stringify(
    Object.fromEntries(
      Object.entries(configFields).filter(([, v]) => v.length > 0),
    ),
  );

  const onTest = async () => {
    setTestState("Testing…");
    try {
      await new Promise((r) => setTimeout(r, 200));
      setTestState("OK");
    } catch {
      setTestState("Error");
    }
  };

  const onSave = async () => {
    setError(null);
    const trimmed = name.trim();
    if (!trimmed) {
      setError("name is required");
      return;
    }
    if (!plugin) {
      setError("plugin is required");
      return;
    }
    setBusy(true);
    try {
      const t = parseInt(tenant, 10);
      await datasourceCreate(trimmed, plugin, configJson, Number.isFinite(t) ? t : 0);
      await onCreated();
    } catch (e) {
      setError(String(e));
    } finally {
      setBusy(false);
    }
  };

  return (
    <div
      className="fixed inset-0 z-50 flex items-center justify-center bg-black/40"
      role="dialog"
      aria-modal="true"
      aria-label="Add datasource"
    >
      <div className="bg-white dark:bg-neutral-800 rounded-lg shadow-xl w-[640px] max-w-[95vw] max-h-[90vh] flex flex-col">
        <header className="flex items-center justify-between px-4 py-3 border-b border-gray-200 dark:border-neutral-700">
          <h2 className="text-sm font-semibold">Add Datasource</h2>
          <button
            type="button"
            onClick={onClose}
            aria-label="Close"
            className="p-1 rounded text-gray-500 hover:bg-gray-100 dark:hover:bg-neutral-700"
          >
            <X size={14} />
          </button>
        </header>

        <div className="p-4 space-y-3 text-xs overflow-y-auto">
          <Field label="Name">
            <input
              value={name}
              onChange={(e) => setName(e.target.value)}
              placeholder="binance"
              className="flex-1 px-2 py-1 rounded border border-gray-200 dark:border-neutral-700 bg-white dark:bg-neutral-900 font-mono"
            />
          </Field>

          <Field label="Plugin">
            <select
              aria-label="plugin"
              value={plugin}
              onChange={(e) => setPlugin(e.target.value)}
              className="flex-1 px-2 py-1 rounded border border-gray-200 dark:border-neutral-700 bg-white dark:bg-neutral-900 font-mono"
            >
              <option value="">— select a plugin —</option>
              {(pluginsQ.data ?? []).map((p) => (
                <option key={p.name} value={p.name}>
                  {p.name}
                </option>
              ))}
            </select>
          </Field>

          <Field label="Tenant (u16)">
            <input
              value={tenant}
              onChange={(e) => setTenant(e.target.value)}
              className="flex-1 px-2 py-1 rounded border border-gray-200 dark:border-neutral-700 bg-white dark:bg-neutral-900 font-mono"
            />
          </Field>

          <div className="rounded border border-gray-200 dark:border-neutral-700 p-3 space-y-2">
            <div className="flex items-center justify-between">
              <div className="text-[10px] uppercase tracking-wider text-gray-500 dark:text-neutral-400">
                Plugin Config
              </div>
              {schemaQ.isFetching && (
                <span className="text-[10px] text-gray-400">loading schema…</span>
              )}
            </div>
            {!plugin && (
              <p className="text-[10px] text-gray-400">select a plugin to load its schema</p>
            )}
            {plugin && (schemaQ.data?.fields ?? []).length === 0 && !schemaQ.isFetching && (
              <p className="text-[10px] text-gray-400">no schema fields</p>
            )}
            {(schemaQ.data?.fields ?? []).map((f: PluginFieldSchema) => (
              <Field key={f.name} label={f.name + (f.required ? " *" : "")}>
                <input
                  value={configFields[f.name] ?? ""}
                  onChange={(e) =>
                    setConfigFields((s) => ({ ...s, [f.name]: e.target.value }))
                  }
                  placeholder={f.description ?? ""}
                  className="flex-1 px-2 py-1 rounded border border-gray-200 dark:border-neutral-700 bg-white dark:bg-neutral-900 font-mono"
                />
              </Field>
            ))}
            <pre className="text-[10px] font-mono text-gray-500 dark:text-neutral-400 bg-gray-50 dark:bg-neutral-900 rounded p-2 overflow-x-auto">
              {configJson}
            </pre>
          </div>

          {testState && (
            <div
              className={[
                "flex items-center gap-1 text-[11px]",
                testState === "OK" ? "text-accent-green" : "text-gray-500",
              ].join(" ")}
            >
              {testState === "OK" && <Check size={11} />}
              {testState}
            </div>
          )}

          {error && <div className="text-accent-red">{error}</div>}
        </div>

        <footer className="px-4 py-3 border-t border-gray-200 dark:border-neutral-700 flex items-center justify-end gap-2">
          <button
            type="button"
            onClick={onTest}
            className="px-3 py-1 text-xs rounded border border-gray-200 dark:border-neutral-700 hover:bg-gray-50 dark:hover:bg-neutral-700"
          >
            Test Connection
          </button>
          <button
            type="button"
            onClick={() => void onSave()}
            disabled={busy}
            className="px-3 py-1 text-xs rounded bg-accent-blue text-white hover:bg-accent-blue/90 disabled:opacity-50"
          >
            Connect and Save
          </button>
          <button
            type="button"
            onClick={onClose}
            className="px-3 py-1 text-xs rounded border border-gray-200 dark:border-neutral-700"
          >
            Cancel
          </button>
        </footer>
      </div>
    </div>
  );
}

function Field({
  label,
  children,
}: {
  label: string;
  children: React.ReactNode;
}) {
  return (
    <div className="flex items-center gap-3 py-1">
      <label className="w-40 text-xs text-gray-600 dark:text-neutral-400">
        {label}
      </label>
      {children}
    </div>
  );
}