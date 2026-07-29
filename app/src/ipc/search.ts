import { invoke } from "@tauri-apps/api/core";

export interface SearchHit {
  kind: string;
  id: string;
  title: string;
  path: string[];
}

export async function searchLocal(query: string): Promise<SearchHit[]> {
  return invoke<SearchHit[]>("search_local", { query });
}

export async function searchServer(query: string): Promise<SearchHit[]> {
  return invoke<SearchHit[]>("search_server", { query });
}
