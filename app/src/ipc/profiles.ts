import { invoke } from "@tauri-apps/api/core";

export interface ProfileView {
  id: number;
  label: string;
  addr: string;
  lastUsedAt: number | null;
  createdAt: number;
}

export async function profilesList(): Promise<ProfileView[]> {
  return invoke<ProfileView[]>("profiles_list");
}

export async function profileSave(label: string, addr: string): Promise<number> {
  return invoke<number>("profile_save", { label, addr });
}

export async function profileRemove(addr: string): Promise<void> {
  await invoke("profile_remove", { addr });
}
