import { create } from "zustand";

import {
  pluginSettingsGet,
  pluginSettingsSet,
  type PluginSettingView,
} from "../ipc";

export interface PluginSettingsStore {
  hydrated: boolean;
  loading: boolean;
  enabled: Record<string, boolean>;
  config: Record<string, string>;
  hydrate(pluginNames: string[]): Promise<void>;
  isEnabled(pluginName: string): boolean;
  setEnabled(pluginName: string, enabled: boolean): Promise<PluginSettingView>;
  setConfig(pluginName: string, configJson: string): void;
  setSettingsFromView(view: PluginSettingView): void;
}

export const usePluginSettings = create<PluginSettingsStore>((set, get) => ({
  hydrated: false,
  loading: false,
  enabled: {},
  config: {},
  async hydrate(pluginNames) {
    const state = get();
    if (state.loading) return;
    const missing = pluginNames.filter((n) => !(n in state.enabled));
    if (missing.length === 0) return;
    set({ loading: true });
    try {
      const nextEnabled: Record<string, boolean> = { ...state.enabled };
      const nextConfig: Record<string, string> = { ...state.config };
      await Promise.all(
        missing.map(async (name) => {
          try {
            const view = await pluginSettingsGet(name);
            nextEnabled[name] = view?.enabled ?? true;
            nextConfig[name] = view?.config_json ?? "{}";
          } catch {
            nextEnabled[name] = true;
            nextConfig[name] = "{}";
          }
        }),
      );
      set({ enabled: nextEnabled, config: nextConfig, hydrated: true, loading: false });
    } catch {
      set({ loading: false });
    }
  },
  isEnabled(pluginName) {
    const v = get().enabled[pluginName];
    return v === undefined ? true : v;
  },
  async setEnabled(pluginName, enabled) {
    const currentConfig = get().config[pluginName] ?? "{}";
    const updated = await pluginSettingsSet(pluginName, enabled, currentConfig);
    set((s) => ({
      enabled: { ...s.enabled, [pluginName]: updated.enabled },
      config: { ...s.config, [pluginName]: updated.config_json },
    }));
    return updated;
  },
  setConfig(pluginName, configJson) {
    set((s) => ({ config: { ...s.config, [pluginName]: configJson } }));
  },
  setSettingsFromView(view) {
    set((s) => ({
      enabled: { ...s.enabled, [view.plugin_name]: view.enabled },
      config: { ...s.config, [view.plugin_name]: view.config_json },
    }));
  },
}));