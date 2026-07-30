import { invoke } from "@tauri-apps/api/core";

export interface ApplicationView {
  id: number;
  name: string;
  enabled: boolean;
  display_order: number;
  tenant: number;
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

export interface ResourceOpView {
  kind: string;
  id: string;
}

export interface FailedResourceView {
  kind: string;
  id: string;
  reason: string;
}

export interface DisableReport {
  application: ApplicationView;
  snapshot: DisableSnapshotView | null;
  succeeded: ResourceOpView[];
  failed: FailedResourceView[];
  skipped: ResourceOpView[];
  pipelines: string[];
  datasources: string[];
  outcome: string;
}

export interface ResourceRehydrationOutcome {
  kind: string;
  name: string;
  result: string;
  detail: string | null;
}

export interface EnableReport {
  application: ApplicationView;
  snapshot: DisableSnapshotView | null;
  succeeded: ResourceOpView[];
  failed: FailedResourceView[];
  skipped: ResourceOpView[];
  rehydrated: ResourceRehydrationOutcome[];
  outcome: string;
}

export async function applicationsList(): Promise<ApplicationView[]> {
  return invoke<ApplicationView[]>("applications_list");
}

export async function applicationCreate(
  name: string,
  tenant?: number | null,
): Promise<ApplicationView> {
  return invoke<ApplicationView>("application_create", { name, tenant: tenant ?? null });
}

export async function applicationSetEnabled(id: number, enabled: boolean): Promise<void> {
  await invoke("application_set_enabled", { id, enabled });
}

export async function applicationEnable(
  id: number,
  addr?: string,
): Promise<EnableReport> {
  return invoke<EnableReport>("application_enable", { id, addr: addr ?? null });
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
