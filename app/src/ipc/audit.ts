import { invoke } from "@tauri-apps/api/core";

export interface AuditEventView {
  id: number;
  timestamp: number;
  actor: string;
  action: string;
  result: string;
  summary: string;
  resource_kind: string | null;
  resource_id: string | null;
  application_id: number | null;
  correlation_id: string | null;
  operation_id: string | null;
  nav_kind: string | null;
  nav_resource_id: string | null;
}

export interface NewAuditEventInput {
  actor: string;
  action: string;
  result: string;
  summary: string;
  resource_kind?: string | null;
  resource_id?: string | null;
  application_id?: number | null;
  correlation_id?: string | null;
  operation_id?: string | null;
  nav_kind?: string | null;
  nav_resource_id?: string | null;
}

export async function auditList(limit = 100): Promise<AuditEventView[]> {
  return invoke<AuditEventView[]>("audit_list", { limit });
}

export async function auditQuery(
  applicationId: number | null,
  limit = 100,
): Promise<AuditEventView[]> {
  return invoke<AuditEventView[]>("audit_query", { applicationId, limit });
}

export async function auditLatest(): Promise<AuditEventView | null> {
  return invoke<AuditEventView | null>("audit_latest");
}

export async function auditRecord(input: NewAuditEventInput): Promise<number> {
  return invoke<number>("audit_record", { ...input });
}