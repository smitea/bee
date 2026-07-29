import { create } from "zustand";

import { tenantGet, tenantSet } from "../ipc/tenant";

export interface TenantStore {
  tenant: number;
  hydrated: boolean;
  refresh(): Promise<void>;
  set(value: number): Promise<number>;
}

export const useTenant = create<TenantStore>((set) => ({
  tenant: 0,
  hydrated: false,
  async refresh() {
    try {
      const tenant = await tenantGet();
      set({ tenant, hydrated: true });
    } catch {
      set({ tenant: 0, hydrated: true });
    }
  },
  async set(value) {
    const validated = await tenantSet(value);
    set({ tenant: validated });
    return validated;
  },
}));