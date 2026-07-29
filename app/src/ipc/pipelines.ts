import { invoke } from "@tauri-apps/api/core";

export interface PipelineSummary {
  id: string;
  name: string;
  dag_hash: string;
  lifecycle: string;
}

export interface PipelineDefinitionView {
  id: number;
  name: string;
  dag_json: string;
  updated_at: number;
}

export async function pipelinesList(addr: string): Promise<PipelineSummary[]> {
  return invoke<PipelineSummary[]>("pipelines_list", { addr });
}

export async function pipelineList(): Promise<PipelineDefinitionView[]> {
  return invoke<PipelineDefinitionView[]>("pipeline_list");
}

export async function pipelineCreate(
  name: string,
  dag_json: string,
): Promise<PipelineDefinitionView> {
  return invoke<PipelineDefinitionView>("pipeline_create", { name, dagJson: dag_json });
}

export async function pipelineGet(id: number): Promise<PipelineDefinitionView | null> {
  return invoke<PipelineDefinitionView | null>("pipeline_get", { id });
}

export async function pipelineDelete(id: number): Promise<void> {
  await invoke("pipeline_delete", { id });
}

export interface PipelineLatestResultView {
  numeric: number;
  label: string;
}

export async function pipelineLatestResult(
  addr: string,
  jobId: number,
): Promise<PipelineLatestResultView | null> {
  return invoke<PipelineLatestResultView | null>("pipeline_latest_result", {
    addr,
    jobId,
  });
}
