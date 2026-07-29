import { create } from "zustand";

import * as ipc from "../ipc/search";
import type { SearchHit } from "../ipc/search";

export interface SearchStoreState {
  query: string;
  loading: boolean;
  results: SearchHit[];
  setQuery(q: string): void;
  runSearchNow(query: string): Promise<void>;
  merge(hits: SearchHit[]): SearchHit[];
}

const DEBOUNCE_MS = 200;

function scoreOf(h: SearchHit, q: string): number {
  const explicit = (h as SearchHit & { _score?: number })._score;
  if (typeof explicit === "number") return explicit;
  if (!q) return 0;
  const ql = q.toLowerCase();
  const title = h.title.toLowerCase();
  if (title === ql) return 1.0;
  if (title.startsWith(ql)) return 0.8;
  if (title.includes(ql)) return 0.6;
  const pathJoin = h.path.join("/").toLowerCase();
  if (pathJoin.includes(ql)) return 0.4;
  return 0.2;
}

let requestSeq = 0;
let debounceHandle: ReturnType<typeof setTimeout> | null = null;

export const useSearch = create<SearchStoreState>((set, get) => ({
  query: "",
  loading: false,
  results: [],
  setQuery(q) {
    set({ query: q });
    if (debounceHandle !== null) {
      clearTimeout(debounceHandle);
      debounceHandle = null;
    }
    if (!q) {
      set({ results: [], loading: false });
      return;
    }
    debounceHandle = setTimeout(() => {
      debounceHandle = null;
      void get().runSearchNow(q);
    }, DEBOUNCE_MS);
  },
  async runSearchNow(query) {
    requestSeq += 1;
    const myId = requestSeq;
    if (!query) {
      set({ results: [], loading: false });
      return;
    }
    set({ loading: true });
    let merged: SearchHit[] = [];
    try {
      const [local, server] = await Promise.all([
        ipc.searchLocal(query).catch(() => [] as SearchHit[]),
        ipc.searchServer(query).catch(() => [] as SearchHit[]),
      ]);
      if (myId !== requestSeq) return;
      merged = get().merge([...local, ...server]);
    } catch {
      if (myId !== requestSeq) return;
      merged = [];
    }
    if (myId !== requestSeq) return;
    set({ results: merged, loading: false });
  },
  merge(hits) {
    const q = get().query;
    return [...hits].sort((a, b) => scoreOf(b, q) - scoreOf(a, q));
  },
}));
