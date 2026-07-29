import { invoke } from "@tauri-apps/api/core";

export interface ClusterProfileView {
  id: number;
  label: string;
  addr: string;
  tenant: number;
  lastUsedAt: number | null;
  createdAt: number;
}

export interface LegacyClusterEntry {
  label: string;
  addr: string;
  tenant?: number | null;
}

export interface MigrationReport {
  inserted: number;
  skipped: string[];
}

export async function clusterProfileList(): Promise<ClusterProfileView[]> {
  return invoke<ClusterProfileView[]>("cluster_profile_list");
}

export async function clusterProfileSave(
  label: string,
  addr: string,
  tenant: number,
): Promise<number> {
  return invoke<number>("cluster_profile_save", { label, addr, tenant });
}

export async function clusterProfileRemove(addr: string): Promise<void> {
  await invoke("cluster_profile_remove", { addr });
}

export async function clusterProfileActivate(addr: string): Promise<ClusterProfileView> {
  return invoke<ClusterProfileView>("cluster_profile_activate", { addr });
}

export async function clusterProfileMigrateLegacy(
  entries: LegacyClusterEntry[],
): Promise<MigrationReport> {
  return invoke<MigrationReport>("cluster_profile_migrate_legacy", { entries });
}