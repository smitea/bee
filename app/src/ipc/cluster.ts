import { invoke } from "@tauri-apps/api/core";

export interface NodeMetricsSummary {
  id: number;
  role: string;
  commit_index: number;
  log_length: number;
}

export interface ClusterMetrics {
  nodes: NodeMetricsSummary[];
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

export interface JobDep {
  upstream_job: number;
  stream: string;
}

export interface TaskRecord {
  task_id: number;
  job_id: number;
  phase_id: number;
  status: string;
  owner_node: number;
  started_at_ms: number;
}

export interface JobDetail {
  job_id: number;
  dag_hash: string;
  lifecycle: string;
  owner_node: number;
  dependencies: JobDep[];
  tasks: TaskRecord[];
}

export async function clusterStatus(addr: string): Promise<ClusterMetrics> {
  return invoke<ClusterMetrics>("cluster_status", { addr });
}

export async function listJobs(addr: string): Promise<JobSummary[]> {
  return invoke<JobSummary[]>("list_jobs", { addr });
}

export async function jobInspect(addr: string, id: number): Promise<JobDetail | null> {
  return invoke<JobDetail | null>("job_inspect", { addr, id });
}
