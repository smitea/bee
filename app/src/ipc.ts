/**
 * Thin IPC wrapper over `invoke()` from `@tauri-apps/api`.
 *
 * Each function maps 1:1 to a `#[tauri::command]` in src-tauri/src/commands.rs.
 */
import { invoke } from "@tauri-apps/api/core";

// Types mirror bee_control::raft types but deserialized to plain JS.
export interface NodeMetrics {
  id: number;
  role: string;
  commit_index: number;
  log_length: number;
}

export interface ClusterMetrics {
  nodes: NodeMetrics[];
  leader_id: number | null;
  term: number;
  commit_index: number;
}

export interface JobSummary {
  job_id: number;
  dag_hash: string;
  lifecycle: string;
  mode: string;
  task_count: number;
  owner_node: number;
}

export interface JobDetail {
  job_id: number;
  dag_hash: string;
  lifecycle: string;
  owner_node: number;
  dependencies: { upstream_job: number; stream: string }[];
  tasks: { task_id: number; phase_id: number; owner_node: number; status: string }[];
}

export interface ConnState {
  addr: string;
  state: string;
  connected: boolean;
}

export async function ping(addr: string): Promise<string> {
  return invoke<string>("ping", { addr });
}

export async function clusterStatus(addr: string): Promise<ClusterMetrics> {
  return invoke<ClusterMetrics>("cluster_status", { addr });
}

export async function listJobs(addr: string): Promise<JobSummary[]> {
  return invoke<JobSummary[]>("list_jobs", { addr });
}

export async function jobInspect(
  addr: string,
  id: number,
): Promise<JobDetail | null> {
  return invoke<JobDetail | null>("job_inspect", { addr, id });
}

export async function connectionState(addr: string): Promise<ConnState> {
  return invoke<ConnState>("connection_state", { addr });
}