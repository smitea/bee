import { invoke } from "@tauri-apps/api/core";

export interface ApplicationView {
  id: number;
  name: string;
  enabled: boolean;
  display_order: number;
  created_at: number;
}

export interface ImportReportView {
  created: string[];
  skipped: string[];
}

export interface DisableSnapshotView {
  application_id: number;
  taken_at: number;
  payload_json: string;
}

export interface DisableReport {
  application: ApplicationView;
  snapshot: DisableSnapshotView;
  pipelines: string[];
  datasources: string[];
}

export async function applicationsList(): Promise<ApplicationView[]> {
  return invoke<ApplicationView[]>("applications_list");
}

export async function applicationCreate(name: string): Promise<ApplicationView> {
  return invoke<ApplicationView>("application_create", { name });
}

export async function applicationSetEnabled(id: number, enabled: boolean): Promise<void> {
  await invoke("application_set_enabled", { id, enabled });
}

export async function applicationEnable(id: number): Promise<ApplicationView> {
  return invoke<ApplicationView>("application_enable", { id });
}

export async function applicationDisable(id: number): Promise<DisableReport> {
  return invoke<DisableReport>("application_disable", { id });
}

export async function applicationDelete(id: number): Promise<void> {
  await invoke("application_delete", { id });
}

export async function applicationExport(
  name: string,
  passphrase: string,
  outPath: string,
): Promise<void> {
  await invoke("application_export", { name, passphrase, outPath });
}

export async function applicationImport(
  filePath: string,
  passphrase: string,
): Promise<ImportReportView> {
  return invoke<ImportReportView>("application_import", {
    filePath,
    passphrase,
  });
}
