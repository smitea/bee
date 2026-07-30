import { invoke } from "@tauri-apps/api/core";

export interface PluginSummary {
  id: string;
  name: string;
  version: string;
  adapters: string[];
  handlers: string[];
}

export interface PluginInfo {
  name: string;
  adapter?: string;
  kind?: string;
}

export interface PluginFieldSchema {
  name: string;
  kind: string;
  required: boolean;
  description: string | null;
}

export interface PluginSchema {
  name: string;
  adapters: Record<string, unknown>;
  fields?: PluginFieldSchema[];
}

export async function pluginList(): Promise<PluginSummary[]> {
  return invoke<PluginSummary[]>("plugin_list");
}

export async function pluginSchema(plugin: string): Promise<PluginSchema> {
  return invoke<PluginSchema>("plugin_schema", { plugin });
}

export async function pluginScanDirectory(path: string): Promise<PluginSummary[]> {
  return invoke<PluginSummary[]>("plugin_scan_directory", { path });
}

export async function pluginDefaultDir(): Promise<string> {
  return invoke<string>("plugin_default_dir");
}

export async function pluginLastDir(): Promise<string | null> {
  return invoke<string | null>("plugin_last_dir");
}

export {
  pluginSettingsGet,
  pluginSettingsSet,
  type PluginSettingView,
} from "./plugin_settings";
