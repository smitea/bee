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

export interface NavTargetView {
  kind: string;
  resourceId: string | null;
  label: string;
}

export function navAction(ev: AuditEventView): NavTargetView | null {
  switch (ev.action) {
    case "cluster.connection.test":
      return {
        kind: "settings.connection",
        resourceId: null,
        label: "Open Settings > Connection",
      };
    case "cluster.connection.activate":
      return null;
    case "pipeline.deploy":
    case "pipeline.job.error":
      if (!ev.resource_id) return null;
      return {
        kind: "application_pipelines",
        resourceId: ev.resource_id,
        label: `Open Pipeline ${ev.resource_id}`,
      };
    case "datasource.create":
    case "datasource.delete":
      if (!ev.resource_id) return null;
      return {
        kind: "application_datasources",
        resourceId: ev.resource_id,
        label: `Open Datasource ${ev.resource_id}`,
      };
    case "application.enable":
    case "application.disable":
      if (!ev.resource_id) return null;
      return {
        kind: "application",
        resourceId: ev.resource_id,
        label: `Open Application ${ev.resource_id}`,
      };
  }
  if (ev.nav_kind) {
    return {
      kind: ev.nav_kind,
      resourceId: ev.nav_resource_id,
      label: `Go to ${ev.nav_kind}`,
    };
  }
  return null;
}

export function categoryOf(action: string): string {
  const idx = action.indexOf(".");
  return idx >= 0 ? action.slice(0, idx) : action;
}