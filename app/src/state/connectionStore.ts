import { create } from "zustand";

import type { ConnStatus, StateView } from "../ipc";
import { connState } from "../ipc";

export interface ConnectionStore {
  addr: string;
  status: ConnStatus;
  hydrated: boolean;
  setAddr(addr: string): void;
  refresh(addr: string): Promise<void>;
}

const initial: Pick<ConnectionStore, "addr" | "status" | "hydrated"> = {
  addr: "127.0.0.1:9999",
  status: { kind: "Connecting" },
  hydrated: false,
};

function reduce(view: StateView, prev: ConnectionStore): ConnectionStore {
  return { ...prev, addr: view.addr, status: view.status };
}

export const useConnection = create<ConnectionStore>((set, get) => ({
  ...initial,
  setAddr(addr) {
    set({ addr });
  },
  async refresh(addr) {
    const target = addr ?? get().addr;
    try {
      const view = await connState(target);
      set((prev) => reduce(view, prev));
    } catch {
      set({ addr: target, status: { kind: "Disconnected" } });
    }
  },
}));
