import { invoke } from "@tauri-apps/api/core";

export interface DatasourceView {
  name: string;
  plugin: string;
  config: string;
  tenant: number;
  created_at: number;
  updated_at: number;
}

export async function datasourceList(): Promise<DatasourceView[]> {
  return invoke<DatasourceView[]>("datasource_list");
}

export async function datasourceCreate(
  name: string,
  plugin: string,
  configJson: string,
  tenant: number,
): Promise<DatasourceView> {
  return invoke<DatasourceView>("datasource_create", {
    name,
    plugin,
    configJson,
    tenant,
  });
}

export async function datasourceDelete(name: string): Promise<void> {
  await invoke("datasource_delete", { name });
}