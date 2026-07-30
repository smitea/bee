import { invoke } from "@tauri-apps/api/core";

export interface TabView {
  id: number;
  kind: string;
  resource_id: string | null;
  title: string;
  pinned: boolean;
  position: number;
}

export interface WorkspaceState {
  activeTabId: number | null;
}

export async function tabsList(): Promise<TabView[]> {
  return invoke<TabView[]>("tabs_list");
}

export async function tabOpen(
  kind: string,
  resourceId: string | null,
  title: string,
): Promise<number> {
  return invoke<number>("tab_open", { kind, resource_id: resourceId, title });
}

export async function tabClose(id: number): Promise<void> {
  await invoke("tab_close", { id });
}

export async function tabCloseOthers(keepId: number): Promise<void> {
  await invoke("tab_close_others", { keepId });
}

export async function tabPin(id: number, pinned: boolean): Promise<void> {
  await invoke("tab_pin", { id, pinned });
}

export async function tabSetActive(id: number | null): Promise<void> {
  await invoke("tab_set_active", { id });
}

export async function workspaceState(): Promise<WorkspaceState> {
  return invoke<WorkspaceState>("workspace_state");
}
