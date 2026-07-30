import { useCallback } from "react";
import { create } from "zustand";

const MAX_HISTORY = 50;

export interface NavigationEntry {
  href: string;
  title: string;
}

interface NavigationState {
  history: NavigationEntry[];
  current: NavigationEntry | null;
  push(entry: NavigationEntry): void;
  pop(): NavigationEntry | null;
  replace(entry: NavigationEntry): void;
  reset(): void;
}

const HOME_ENTRY: NavigationEntry = { href: "/", title: "Home" };

export const useNavigationStore = create<NavigationState>((set, get) => ({
  history: [],
  current: HOME_ENTRY,
  push(entry) {
    const cur = get().current;
    if (cur && cur.href === entry.href) return;
    const next = [...get().history, cur].filter(Boolean) as NavigationEntry[];
    while (next.length > MAX_HISTORY) next.shift();
    set({ history: next, current: entry });
  },
  pop() {
    const prev = get().history[get().history.length - 1];
    if (!prev) return null;
    const nextHistory = get().history.slice(0, -1);
    set({ history: nextHistory, current: prev });
    return prev;
  },
  replace(entry) {
    set({ current: entry });
  },
  reset() {
    set({ history: [], current: HOME_ENTRY });
  },
}));

export interface UseNavigation {
  current: NavigationEntry | null;
  history: NavigationEntry[];
  push: (entry: NavigationEntry) => void;
  back: () => NavigationEntry | null;
  replace: (entry: NavigationEntry) => void;
  reset: () => void;
  canGoBack: boolean;
}

export function useNavigation(): UseNavigation {
  const current = useNavigationStore((s) => s.current);
  const history = useNavigationStore((s) => s.history);
  const pushFn = useNavigationStore((s) => s.push);
  const popFn = useNavigationStore((s) => s.pop);
  const replaceFn = useNavigationStore((s) => s.replace);
  const resetFn = useNavigationStore((s) => s.reset);
  const back = useCallback(() => popFn(), [popFn]);
  return {
    current,
    history,
    push: pushFn,
    back,
    replace: replaceFn,
    reset: resetFn,
    canGoBack: history.length > 0,
  };
}