import { create } from "zustand";

import * as ipc from "../ipc/audit";
import type { AuditEventView } from "../ipc/audit";

export interface AuditStore {
  events: AuditEventView[];
  loaded: boolean;
  refresh(limit?: number): Promise<void>;
  latest(): Promise<AuditEventView | null>;
}

export const useAudit = create<AuditStore>((set) => ({
  events: [],
  loaded: false,
  async refresh(limit = 100) {
    const events = await ipc.auditList(limit);
    set({ events, loaded: true });
  },
  async latest() {
    const ev = await ipc.auditLatest();
    if (ev) {
      set((s) => {
        const filtered = s.events.filter((e) => e.id !== ev.id);
        return { events: [ev, ...filtered] };
      });
    }
    return ev;
  },
}));

export function summary(ev: AuditEventView): string {
  return ev.summary;
}

export function navTarget(
  ev: AuditEventView,
): { kind: string; resourceId: string | null } | null {
  if (!ev.nav_kind) return null;
  return { kind: ev.nav_kind, resourceId: ev.nav_resource_id };
}