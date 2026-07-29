import { useEffect, useState } from "react";
import { LayoutDashboard, Plus, Trash2 } from "lucide-react";

import { DashboardPanel } from "./DashboardPanel";
import { useConnection } from "../state/connectionStore";
import { useApplications } from "../state/applicationsStore";

interface PanelDef {
  id: string;
  label: string;
  jobId: number;
}

const FALLBACK_PANELS: PanelDef[] = [
  { id: "p1", label: "Default panel", jobId: 1 },
];

const LS_KEY_PREFIX = "bee-client.dashboard.";

interface Props {
  applicationId: number;
}

function readPanels(applicationId: number): PanelDef[] | null {
  if (typeof localStorage === "undefined") return null;
  const raw = localStorage.getItem(LS_KEY_PREFIX + applicationId);
  if (!raw) return null;
  try {
    const parsed = JSON.parse(raw);
    if (Array.isArray(parsed) && parsed.every((p) => typeof p === "object")) {
      return parsed as PanelDef[];
    }
    return null;
  } catch {
    return null;
  }
}

function writePanels(applicationId: number, panels: PanelDef[]): void {
  if (typeof localStorage === "undefined") return;
  localStorage.setItem(LS_KEY_PREFIX + applicationId, JSON.stringify(panels));
}

export function DashboardPage({ applicationId }: Props) {
  const addr = useConnection((s) => s.addr);
  const applications = useApplications((s) => s.items);
  const [panels, setPanels] = useState<PanelDef[]>([]);
  const [draftLabel, setDraftLabel] = useState("");
  const [draftJobId, setDraftJobId] = useState("");

  useEffect(() => {
    const stored = readPanels(applicationId);
    setPanels(stored ?? FALLBACK_PANELS);
  }, [applicationId]);

  const addPanel = () => {
    const label = draftLabel.trim();
    const jobIdNum = Number(draftJobId);
    if (!label || !Number.isFinite(jobIdNum)) return;
    const next: PanelDef[] = [
      ...panels,
      { id: `p-${Date.now()}`, label, jobId: jobIdNum },
    ];
    setPanels(next);
    writePanels(applicationId, next);
    setDraftLabel("");
    setDraftJobId("");
  };

  const removePanel = (id: string) => {
    const next = panels.filter((p) => p.id !== id);
    setPanels(next);
    writePanels(applicationId, next);
  };

  const application = applications.find((a) => a.id === applicationId);

  return (
    <div className="space-y-4">
      <header className="flex items-center justify-between">
        <div>
          <h1 className="text-xl font-semibold flex items-center gap-2">
            <LayoutDashboard size={18} className="text-accent-blue" />
            Dashboard{application ? ` · ${application.name}` : ""}
          </h1>
          <p className="text-xs text-gray-500 dark:text-neutral-400">
            live panels polling server (slice 5 polling surface)
          </p>
        </div>
      </header>

      <div className="grid grid-cols-1 md:grid-cols-2 xl:grid-cols-3 gap-3">
        {panels.map((p) => (
          <div key={p.id} className="relative group">
            <DashboardPanel addr={addr} jobId={p.jobId} label={p.label} />
            <button
              type="button"
              onClick={() => removePanel(p.id)}
              aria-label={`remove panel ${p.label}`}
              className="absolute right-2 top-2 opacity-0 group-hover:opacity-100 p-1 text-gray-400 hover:text-accent-red"
            >
              <Trash2 size={11} />
            </button>
          </div>
        ))}
        {panels.length === 0 && (
          <p className="text-xs text-gray-400 col-span-full">no panels yet</p>
        )}
      </div>

      <section className="bg-white dark:bg-neutral-800 rounded-lg border border-gray-200 dark:border-neutral-700 p-4 space-y-3">
        <div className="text-[10px] uppercase tracking-wider text-gray-500 dark:text-neutral-400">
          Add a panel
        </div>
        <div className="grid grid-cols-1 md:grid-cols-3 gap-3 text-xs">
          <label className="flex flex-col gap-1">
            <span className="text-gray-500 dark:text-neutral-400">Label</span>
            <input
              value={draftLabel}
              onChange={(e) => setDraftLabel(e.target.value)}
              placeholder="Open interest"
              className="px-2 py-1 rounded border border-gray-200 dark:border-neutral-700 bg-white dark:bg-neutral-900"
            />
          </label>
          <label className="flex flex-col gap-1">
            <span className="text-gray-500 dark:text-neutral-400">Pipeline Job id</span>
            <input
              value={draftJobId}
              onChange={(e) => setDraftJobId(e.target.value)}
              placeholder="1"
              className="px-2 py-1 rounded border border-gray-200 dark:border-neutral-700 bg-white dark:bg-neutral-900 font-mono"
            />
          </label>
          <div className="flex items-end">
            <button
              type="button"
              onClick={addPanel}
              disabled={!draftLabel.trim() || !draftJobId.trim()}
              className="flex items-center gap-1 px-3 py-1.5 rounded bg-accent-blue text-white hover:bg-accent-blue/90 disabled:opacity-50"
            >
              <Plus size={12} />
              Add panel
            </button>
          </div>
        </div>
      </section>
    </div>
  );
}
