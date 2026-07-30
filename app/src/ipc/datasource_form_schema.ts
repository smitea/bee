import { invoke } from "@tauri-apps/api/core";

export type JsonSchemaType = "string" | "integer" | "number" | "boolean" | "object" | "array";

export interface JsonSchema {
  type: JsonSchemaType;
  required?: boolean | string[];
  description?: string | null;
  default?: unknown;
  enum?: unknown[];
  properties?: Record<string, JsonSchema>;
  items?: JsonSchema;
  minimum?: number;
  maximum?: number;
}

export interface JsonSchemaField {
  name: string;
  schema: JsonSchema;
  required: boolean;
}

export function jsonSchemaToFields(schema: JsonSchema): JsonSchemaField[] {
  const props = schema.properties ?? {};
  const requiredRaw = schema.required;
  const required = new Set<string>(
    Array.isArray(requiredRaw) ? requiredRaw : [],
  );
  return Object.entries(props).map(([name, s]) => ({
    name,
    schema: s,
    required: required.has(name),
  }));
}

export interface DatasourceFormSchema {
  plugin_name: string;
  adapter: string | null;
  fields: JsonSchemaField[];
}

export async function datasourceFormSchema(pluginName: string): Promise<DatasourceFormSchema> {
  return invoke<DatasourceFormSchema>("datasource_form_schema", { pluginName });
}