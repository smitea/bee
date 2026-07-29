import { invoke } from "@tauri-apps/api/core";

import type { StateView } from "./shared";

export async function ping(addr: string): Promise<string> {
  return invoke<string>("ping", { addr });
}

export async function getDefaultAddr(): Promise<string> {
  return invoke<string>("get_default_addr");
}

export async function setAddr(addr: string): Promise<StateView> {
  return invoke<StateView>("set_addr", { addr });
}

export async function testConnection(addr: string): Promise<StateView> {
  return invoke<StateView>("test_connection", { addr });
}

export async function connState(addr: string): Promise<StateView> {
  return invoke<StateView>("conn_state", { addr });
}
