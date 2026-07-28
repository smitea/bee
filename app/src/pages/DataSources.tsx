import { useState } from "react";
import { Database, Plus } from "lucide-react";
import { useStore } from "../state/store";

interface Datasource {
  name: string;
  adapter: string;
  version_spec: string;
  status: "Active" | "Paused" | "Disabled";
  plugin_id: string;
  config: string;
  tenant: number;
}

// MVP: client-side in-process DatasourceRegistry (mirrors the iced impl).
// Production wires through AdminServer. See S-Tauri.2.
function registry(): {
  list: () => Datasource[];
  create: (d: Omit<Datasource, "plugin_id" | "status">) => Datasource;
  pause: (name: string) => void;
  resume: (name: string) => void;
  remove: (name: string) => void;
} {
  const key = "bee-gui.datasources";
  const load = (): Datasource[] => {
    if (typeof localStorage === "undefined") return [];
    try {
      return JSON.parse(localStorage.getItem(key) || "[]");
    } catch {
      return [];
    }
  };
  const save = (ds: Datasource[]) =>
    localStorage.setItem(key, JSON.stringify(ds));
  return {
    list: load,
    create: (d) => {
      const ds: Datasource = {
        ...d,
        plugin_id: `mvp-${d.adapter}-${d.version_spec}`.replace(
          /[^a-z0-9]/gi,
          "",
        ),
        status: "Active",
      };
      const all = load();
      all.push(ds);
      save(all);
      return ds;
    },
    pause: (name) =>
      save(
        load().map((d) =>
          d.name === name ? { ...d, status: "Paused" as const } : d,
        ),
      ),
    resume: (name) =>
      save(
        load().map((d) =>
          d.name === name ? { ...d, status: "Active" as const } : d,
        ),
      ),
    remove: (name) => save(load().filter((d) => d.name !== name)),
  };
}

export function DataSources() {
  const addr = useStore((s) => s.addr);
  const [version, setVersion] = useState(0);
  const r = registry();
  const all = r.list();

  const [form, setForm] = useState({
    name: "",
    adapter: "",
    version: "^1.0",
    config: "{}",
    tenant: "0",
  });
  const [error, setError] = useState<string | null>(null);
  const [inspect, setInspect] = useState<string | null>(null);

  const handleCreate = () => {
    setError(null);
    if (!form.name.trim() || !form.adapter.trim()) {
      setError("name and adapter are required");
      return;
    }
    if (all.some((d) => d.name === form.name.trim())) {
      setError(`datasource '${form.name}' already exists`);
      return;
    }
    r.create({
      name: form.name.trim(),
      adapter: form.adapter.trim(),
      version_spec: form.version.trim() || "*",
      config: form.config,
      tenant: parseInt(form.tenant, 10) || 0,
    });
    setForm({ name: "", adapter: "", version: "^1.0", config: "{}", tenant: "0" });
    setVersion(version + 1);
  };

  const statusColor = (s: Datasource["status"]) =>
    s === "Active"
      ? "bg-accent-green"
      : s === "Paused"
        ? "bg-accent-orange"
        : "bg-gray-400";

  return (
    <div className="space-y-6">
      <h1 className="text-xl font-semibold">Data Sources</h1>

      <div className="bg-white dark:bg-neutral-800 rounded-lg border border-gray-200 dark:border-neutral-700">
        <h2 className="px-4 py-3 text-sm font-medium border-b border-gray-200 dark:border-neutral-700">
          Create Datasource
        </h2>
        <div className="p-4 space-y-2">
          <Field label="Name">
            <input
              value={form.name}
              onChange={(e) => setForm({ ...form, name: e.target.value })}
              placeholder="binance"
              className="flex-1 px-3 py-1.5 text-xs rounded-md border border-gray-200 dark:border-neutral-700 bg-white dark:bg-neutral-900"
            />
          </Field>
          <Field label="Adapter">
            <input
              value={form.adapter}
              onChange={(e) => setForm({ ...form, adapter: e.target.value })}
              placeholder="binance_subscribe"
              className="flex-1 px-3 py-1.5 text-xs rounded-md border border-gray-200 dark:border-neutral-700 bg-white dark:bg-neutral-900"
            />
          </Field>
          <Field label="Plugin Version (SemVer)">
            <input
              value={form.version}
              onChange={(e) => setForm({ ...form, version: e.target.value })}
              placeholder="^1.0"
              className="flex-1 px-3 py-1.5 text-xs rounded-md border border-gray-200 dark:border-neutral-700 bg-white dark:bg-neutral-900"
            />
          </Field>
          <Field label="Config (JSON)">
            <input
              value={form.config}
              onChange={(e) => setForm({ ...form, config: e.target.value })}
              className="flex-1 px-3 py-1.5 text-xs rounded-md border border-gray-200 dark:border-neutral-700 bg-white dark:bg-neutral-900 font-mono"
            />
          </Field>
          <Field label="Tenant (u16)">
            <input
              value={form.tenant}
              onChange={(e) => setForm({ ...form, tenant: e.target.value })}
              className="flex-1 px-3 py-1.5 text-xs rounded-md border border-gray-200 dark:border-neutral-700 bg-white dark:bg-neutral-900"
            />
          </Field>
          <div className="pt-2 flex items-center gap-2">
            <button
              onClick={handleCreate}
              className="flex items-center gap-1.5 px-3 py-1.5 text-xs rounded-md bg-accent-blue text-white hover:bg-accent-blue/90"
            >
              <Plus size={14} />
              Create
            </button>
            {error && (
              <span className="text-xs text-accent-red">{error}</span>
            )}
          </div>
        </div>
      </div>

      <div className="bg-white dark:bg-neutral-800 rounded-lg border border-gray-200 dark:border-neutral-700">
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
              <p className="mt-2 text-sm">no datasources — create one above</p>
            </div>
          ) : (
            <table className="w-full text-xs">
              <thead>
                <tr className="text-[10px] uppercase tracking-wider text-gray-500 dark:text-neutral-400">
                  <th className="text-left font-medium pb-2 pr-4">Name</th>
                  <th className="text-left font-medium pb-2 pr-4">Status</th>
                  <th className="text-left font-medium pb-2 pr-4">Adapter</th>
                  <th className="text-left font-medium pb-2 pr-4">Version</th>
                  <th className="text-left font-medium pb-2 pr-4">Actions</th>
                </tr>
              </thead>
              <tbody>
                {all.map((d) => (
                  <tr
                    key={d.name}
                    className="border-t border-gray-100 dark:border-neutral-800"
                  >
                    <td className="py-2 pr-4 font-mono">
                      <span
                        className={`inline-block w-2 h-2 rounded-full mr-2 align-middle ${statusColor(d.status)}`}
                      />
                      {d.name}
                    </td>
                    <td className="py-2 pr-4 text-gray-600 dark:text-neutral-300">
                      {d.status}
                    </td>
                    <td className="py-2 pr-4 font-mono">{d.adapter}</td>
                    <td className="py-2 pr-4 font-mono">{d.version_spec}</td>
                    <td className="py-2 space-x-1">
                      <ActionButton onClick={() => setInspect(d.name)}>
                        Inspect
                      </ActionButton>
                      <ActionButton
                        onClick={() => {
                          r.pause(d.name);
                          setVersion(version + 1);
                        }}
                      >
                        Pause
                      </ActionButton>
                      <ActionButton
                        onClick={() => {
                          r.resume(d.name);
                          setVersion(version + 1);
                        }}
                      >
                        Resume
                      </ActionButton>
                      <ActionButton
                        onClick={() => {
                          r.remove(d.name);
                          setVersion(version + 1);
                        }}
                      >
                        Delete
                      </ActionButton>
                    </td>
                  </tr>
                ))}
              </tbody>
            </table>
          )}
        </div>
      </div>

      {inspect && (
        <div className="bg-white dark:bg-neutral-800 rounded-lg border border-gray-200 dark:border-neutral-700">
          <h2 className="px-4 py-3 text-sm font-medium border-b border-gray-200 dark:border-neutral-700 flex items-center justify-between">
            <span>Inspect: {inspect}</span>
            <button
              onClick={() => setInspect(null)}
              className="px-2 py-1 text-xs rounded-md border border-gray-200 dark:border-neutral-700 hover:bg-gray-50 dark:hover:bg-neutral-700"
            >
              Close
            </button>
          </h2>
          <div className="p-4 text-xs space-y-1">
            {(() => {
              const d = all.find((x) => x.name === inspect);
              if (!d) return <p>not found</p>;
              return (
                <>
                  <Row label="adapter">{d.adapter}</Row>
                  <Row label="plugin_id">{d.plugin_id}</Row>
                  <Row label="version_spec">{d.version_spec}</Row>
                  <Row label="status">{d.status}</Row>
                  <Row label="tenant">{d.tenant}</Row>
                  <Row label="config">
                    <code className="text-[10px]">{d.config}</code>
                  </Row>
                </>
              );
            })()}
          </div>
        </div>
      )}

      <p className="text-xs text-gray-500 dark:text-neutral-400">
        S-2 (MVP): in-process DatasourceRegistry at addr {addr}. Production
        wires through AdminServer RPC + Raft KV (S30.x).
      </p>
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

function Row({ label, children }: { label: string; children: React.ReactNode }) {
  return (
    <div className="flex items-start gap-3 py-0.5">
      <span className="w-32 text-gray-600 dark:text-neutral-400">{label}:</span>
      <span className="font-mono text-[11px]">{children}</span>
    </div>
  );
}

function ActionButton({
  onClick,
  children,
}: {
  onClick: () => void;
  children: React.ReactNode;
}) {
  return (
    <button
      onClick={onClick}
      className="px-2 py-0.5 text-[11px] rounded border border-gray-200 dark:border-neutral-700 hover:bg-gray-50 dark:hover:bg-neutral-700"
    >
      {children}
    </button>
  );
}