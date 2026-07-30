import { invoke } from "@tauri-apps/api/core";

export interface PluginSettingView {
  plugin_name: string;
  enabled: boolean;
  config_json: string;
  updated_at: number;
}

export async function pluginSettingsGet(name: string): Promise<PluginSettingView | null> {
  return invoke<PluginSettingView | null>("plugin_settings_get", { name });
}

export async function pluginSettingsSet(
  name: string,
  enabled: boolean,
  configJson: string,
): Promise<PluginSettingView> {
  return invoke<PluginSettingView>("plugin_settings_set", {
    name,
    enabled,
    configJson,
  });
}