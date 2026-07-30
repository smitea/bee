import { create } from "zustand";

export type ThemeKind = "light" | "dark";

export type SettingsSection =
  | "client"
  | "connection"
  | "tenant"
  | "appearance"
  | "logging"
  | "diagnostics"
  | "raft"
  | "kv"
  | "scheduling"
  | "plugins"
  | "security";

export interface UiState {
  theme: ThemeKind;
  settingsOpen: boolean;
  settingsSection: SettingsSection;
  openSettings(section?: SettingsSection): void;
  closeSettings(): void;
  setTheme(t: ThemeKind): void;
  toggleTheme(): void;
}

const LS_THEME = "bee-client.theme";

function lsGet<T>(key: string, fallback: T): T {
  if (typeof localStorage === "undefined") return fallback;
  const v = localStorage.getItem(key);
  if (v === null) return fallback;
  try {
    return JSON.parse(v) as T;
  } catch {
    return fallback;
  }
}

function lsSet(key: string, v: unknown) {
  if (typeof localStorage === "undefined") return;
  try {
    localStorage.setItem(key, JSON.stringify(v));
  } catch {}
}

export const useUi = create<UiState>((set, get) => ({
  theme: lsGet<ThemeKind>(LS_THEME, "light"),
  settingsOpen: false,
  settingsSection: "connection",
  openSettings: (section) =>
    set({ settingsOpen: true, settingsSection: section ?? "connection" }),
  closeSettings: () => set({ settingsOpen: false }),
  setTheme: (t) => {
    lsSet(LS_THEME, t);
    set({ theme: t });
    if (typeof document !== "undefined") {
      document.documentElement.classList.toggle("dark", t === "dark");
    }
  },
  toggleTheme: () => {
    const cur = get().theme;
    get().setTheme(cur === "light" ? "dark" : "light");
  },
}));