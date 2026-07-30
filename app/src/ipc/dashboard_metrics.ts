import { invoke } from "@tauri-apps/api/core";

export interface DashboardMetricView {
  dashboard_id: number;
  panel_id: string;
  pipeline_job_id: number | null;
  source_field: string;
  widget_kind: string;
  chart_config_json: string;
  updated_at: number;
}

export async function dashboardMetricGet(
  dashboardId: number,
  panelId: string,
): Promise<DashboardMetricView | null> {
  return invoke<DashboardMetricView | null>("dashboard_metric_get", {
    dashboardId,
    panelId,
  });
}

export async function dashboardMetricList(
  dashboardId: number,
): Promise<DashboardMetricView[]> {
  return invoke<DashboardMetricView[]>("dashboard_metric_list", { dashboardId });
}

export async function dashboardMetricSave(
  dashboardId: number,
  panelId: string,
  pipelineJobId: number | null,
  sourceField: string,
  widgetKind: string,
  chartConfigJson: string,
): Promise<DashboardMetricView> {
  return invoke<DashboardMetricView>("dashboard_metric_save", {
    dashboardId,
    panelId,
    pipelineJobId,
    sourceField,
    widgetKind,
    chartConfigJson,
  });
}

export async function dashboardMetricDelete(
  dashboardId: number,
  panelId: string,
): Promise<void> {
  await invoke("dashboard_metric_delete", { dashboardId, panelId });
}