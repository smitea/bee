import { invoke } from "@tauri-apps/api/core";

export interface DashboardView {
  application_id: number;
  layout_json: string;
  updated_at: number;
}

export interface DashboardLayout {
  panels: DashboardPanel[];
}

export interface DashboardPanel {
  id: string;
  kind: "kline" | "active_jobs" | "tasks_per_sec" | "cpu" | "pipeline_status" | "audit_feed" | "cluster_topology";
  x: number;
  y: number;
  w: number;
  h: number;
  title: string;
  job_id?: number;
}

export async function dashboardGet(applicationId: number): Promise<DashboardView | null> {
  return invoke<DashboardView | null>("dashboard_get", { applicationId });
}

export async function dashboardSave(
  applicationId: number,
  layoutJson: string,
): Promise<DashboardView> {
  return invoke<DashboardView>("dashboard_save", { applicationId, layoutJson });
}
