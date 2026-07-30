import { invoke } from "@tauri-apps/api/core";

export interface PipelineDumpView {
  pipeline_id: number;
  dump_json: string;
  created_at: number;
}

export async function pipelineDumpList(pipelineId: number): Promise<PipelineDumpView[]> {
  return invoke<PipelineDumpView[]>("pipeline_dump_list", { pipelineId });
}

export async function pipelineDumpRecord(
  pipelineId: number,
  dumpJson: string,
): Promise<PipelineDumpView> {
  return invoke<PipelineDumpView>("pipeline_dump_record", {
    pipelineId,
    dumpJson,
  });
}