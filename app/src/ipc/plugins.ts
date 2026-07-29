import { invoke } from "@tauri-apps/api/core";

export interface PluginInfo {
  name: string;
  adapter: string;
  kind: string;
}

export interface PluginFieldSchema {
  name: string;
  kind: string;
  required: boolean;
  description: string | null;
}

export interface PluginSchema {
  name: string;
  fields: PluginFieldSchema[];
}

export async function pluginList(): Promise<PluginInfo[]> {
  return invoke<PluginInfo[]>("plugin_list");
}

export async function pluginSchema(plugin: string): Promise<PluginSchema> {
  return invoke<PluginSchema>("plugin_schema", { plugin });
}