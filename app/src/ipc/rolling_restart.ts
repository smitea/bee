import { invoke } from "@tauri-apps/api/core";

export interface NodeAddr {
  id: string;
  addr: string;
}

export interface RollingRestartPlan {
  nodes: NodeAddr[];
  batch_size: number;
  health_timeout_ms: number;
}

export interface StepResult {
  restarted: string[];
  failed: string | null;
  done: boolean;
  next_step: number;
}

export async function rollingRestartApply(
  addr: string,
  nodes: NodeAddr[],
): Promise<RollingRestartPlan> {
  return invoke<RollingRestartPlan>("rolling_restart_apply", { addr, nodes });
}
