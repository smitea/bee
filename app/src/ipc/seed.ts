import { invoke } from "@tauri-apps/api/core";

export interface SeedReportView {
  created: boolean;
  application_id: number | null;
  pipeline_id: number | null;
  datasource_name: string | null;
  audit_events: number;
}

export async function seedDemo(): Promise<SeedReportView> {
  return invoke<SeedReportView>("seed_demo");
}
