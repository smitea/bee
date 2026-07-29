import { invoke } from "@tauri-apps/api/core";

export interface Setting {
  key: string;
  value: string;
}

export async function settingsGet(key: string): Promise<string | null> {
  return invoke<string | null>("settings_get", { key });
}

export async function settingsPut(key: string, value: string): Promise<void> {
  await invoke("settings_put", { key, value });
}

export async function settingsList(): Promise<Setting[]> {
  return invoke<Setting[]>("settings_list");
}
