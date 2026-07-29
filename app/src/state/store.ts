import { create } from "zustand";
import { setAddr as ipcSetAddr, getDefaultAddr } from "../ipc";

export type Tab = "dashboard" | "dataSources" | "pipelines" | "settings";

export type ThemeKind = "light" | "dark";

export interface AppState {
  tab: Tab;
  theme: ThemeKind;
  addr: string;
  logLevel: "debug" | "info" | "warn" | "error";
  setTab: (t: Tab) => void;
  setTheme: (t: ThemeKind) => void;
  toggleTheme: () => void;
  setAddr: (a: string) => void;
  setLogLevel: (l: AppState["logLevel"]) => void;
}

const LS_ADDR = "bee-gui.addr";
const LS_THEME = "bee-gui.theme";
const LS_TAB = "bee-gui.tab";

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
  } catch {
    // ignore quota errors
  }
}

const initialAddr = lsGet<string | null>(LS_ADDR, null);

export const useStore = create<AppState>((set, get) => ({
  tab: lsGet<Tab>(LS_TAB, "dashboard"),
  theme: lsGet<ThemeKind>(LS_THEME, "light"),
  addr: initialAddr ?? "127.0.0.1:9999",
  logLevel: "info",
  setTab: (t) => {
    lsSet(LS_TAB, t);
    set({ tab: t });
  },
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
  setAddr: (a) => {
    lsSet(LS_ADDR, a);
    set({ addr: a });
    ipcSetAddr(a).catch((err) => {
      console.warn("[bee-gui] ipcSetAddr error:", err);
    });
  },
  setLogLevel: (l) => set({ logLevel: l }),
}));

// If localStorage didn't have an address, query backend for BEE_ADMIN_ADDR env var or fallback
if (initialAddr === null) {
  getDefaultAddr()
    .then((defaultAddr) => {
      if (typeof localStorage !== "undefined" && localStorage.getItem(LS_ADDR) === null) {
        useStore.getState().setAddr(defaultAddr);
      }
    })
    .catch(() => {});
}