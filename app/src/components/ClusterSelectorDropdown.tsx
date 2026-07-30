import { useEffect, useRef, useState } from "react";
import { useQuery, useQueryClient } from "@tanstack/react-query";
import { ChevronDown, Server } from "lucide-react";

import {
  clusterProfileList,
  clusterProfileActivate,
  type ClusterProfileView,
} from "../ipc/clusters";
import { setAddr } from "../ipc/connection";
import { useConnection } from "../state/connectionStore";
import { ClusterStatusDot } from "./ClusterStatusDot";

export function ClusterSelectorDropdown() {
  const qc = useQueryClient();
  const [open, setOpen] = useState(false);
  const ref = useRef<HTMLDivElement | null>(null);

  const addr = useConnection((s) => s.addr);
  const status = useConnection((s) => s.status);
  const setStoreAddr = useConnection((s) => s.setAddr);

  const listQ = useQuery<ClusterProfileView[]>({
    queryKey: ["cluster-profiles"],
    queryFn: () => clusterProfileList(),
  });

  useEffect(() => {
    if (!open) return;
    const onDoc = (e: MouseEvent) => {
      if (ref.current && !ref.current.contains(e.target as Node)) setOpen(false);
    };
    const onKey = (e: KeyboardEvent) => {
      if (e.key === "Escape") setOpen(false);
    };
    document.addEventListener("mousedown", onDoc);
    document.addEventListener("keydown", onKey);
    return () => {
      document.removeEventListener("mousedown", onDoc);
      document.removeEventListener("keydown", onKey);
    };
  }, [open]);

  const onActivate = async (profileAddr: string) => {
    try {
      const view = await clusterProfileActivate(profileAddr);
      await setAddr(view.addr);
      setStoreAddr(view.addr);
      await qc.invalidateQueries({ queryKey: ["cluster-profiles"] });
      setOpen(false);
    } catch {
      /* surfaced by setAddr/state */
    }
  };

  const all = listQ.data ?? [];
  const active = all.find((p) => p.addr.trim() === addr.trim()) ?? null;
  const label = active ? active.label : addr;

  return (
    <div ref={ref} className="relative">
      <button
        type="button"
        aria-label="Select cluster"
        aria-haspopup="menu"
        aria-expanded={open}
        onClick={() => setOpen((s) => !s)}
        className="flex items-center gap-1.5 px-2 py-1 rounded text-gray-600 dark:text-neutral-300 hover:bg-gray-100 dark:hover:bg-neutral-700"
        title="Switch cluster"
      >
        <ClusterStatusDot
          profileAddr={addr}
          activeAddr={addr}
          status={status}
          size={6}
        />
        <Server size={12} />
        <span className="font-mono text-[11px]">{label}</span>
        <ChevronDown size={10} />
      </button>
      {open && (
        <div
          role="menu"
          className="absolute left-0 top-full mt-1 z-40 min-w-[18rem] rounded-md border border-gray-200 dark:border-neutral-700 bg-white dark:bg-neutral-800 shadow-lg text-xs"
        >
          <div className="px-3 py-2 text-[10px] font-semibold uppercase tracking-wider text-gray-500 dark:text-neutral-400 border-b border-gray-200 dark:border-neutral-700">
            Clusters ({all.length})
          </div>
          <div className="max-h-72 overflow-y-auto py-1">
            {all.length === 0 && (
              <p className="px-3 py-2 text-[11px] text-gray-400">no saved clusters</p>
            )}
            {all.map((p) => {
              const isActive = p.addr.trim() === addr.trim();
              return (
                <button
                  key={p.id}
                  type="button"
                  role="menuitem"
                  onClick={() => void onActivate(p.addr)}
                  className={[
                    "w-full flex items-center gap-2 px-3 py-1.5 text-left",
                    isActive
                      ? "bg-accent-blue/10"
                      : "hover:bg-gray-100 dark:hover:bg-neutral-700",
                  ].join(" ")}
                >
                  <ClusterStatusDot
                    profileAddr={p.addr}
                    activeAddr={addr}
                    status={isActive ? status : { kind: "Disconnected" }}
                  />
                  <div className="flex-1 min-w-0">
                    <div className="truncate">{p.label}</div>
                    <div className="font-mono text-[10px] text-gray-400 truncate">
                      {p.addr} · t{p.tenant}
                    </div>
                  </div>
                </button>
              );
            })}
          </div>
        </div>
      )}
    </div>
  );
}