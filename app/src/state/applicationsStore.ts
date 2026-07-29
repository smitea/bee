import { create } from "zustand";

import * as ipc from "../ipc/applications";
import type { ApplicationView } from "../ipc/applications";

export interface ApplicationsStore {
  items: ApplicationView[];
  loaded: boolean;
  refresh(): Promise<void>;
  create(name: string): Promise<ApplicationView>;
  setEnabled(id: number, enabled: boolean): Promise<void>;
  delete(id: number): Promise<void>;
}

export const useApplications = create<ApplicationsStore>((set) => ({
  items: [],
  loaded: false,
  async refresh() {
    const items = await ipc.applicationsList();
    set({ items, loaded: true });
  },
  async create(name) {
    const created = await ipc.applicationCreate(name);
    set((s) => ({ items: [...s.items, created] }));
    return created;
  },
  async setEnabled(id, enabled) {
    await ipc.applicationSetEnabled(id, enabled);
    set((s) => ({
      items: s.items.map((a) => (a.id === id ? { ...a, enabled } : a)),
    }));
  },
  async delete(id) {
    await ipc.applicationDelete(id);
    set((s) => ({ items: s.items.filter((a) => a.id !== id) }));
  },
}));