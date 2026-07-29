import { invoke } from "@tauri-apps/api/core";

export async function tenantGet(): Promise<number> {
  return invoke<number>("tenant_get");
}

export async function tenantSet(value: number): Promise<number> {
  return invoke<number>("tenant_set", { value });
}