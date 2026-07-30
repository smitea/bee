import { useMemo } from "react";

import type { JsonSchema, JsonSchemaField } from "../ipc/datasource_form_schema";
import { jsonSchemaToFields } from "../ipc/datasource_form_schema";

export type SchemaValue = string | number | boolean | null | SchemaObject | SchemaValue[];
export interface SchemaObject {
  [k: string]: SchemaValue;
}

interface Props {
  schema: JsonSchema | null;
  value: SchemaObject;
  onChange(v: SchemaObject): void;
  disabled?: boolean;
}

function defaultForSchema(s: JsonSchema): SchemaValue {
  if (s.default !== undefined) return s.default as SchemaValue;
  switch (s.type) {
    case "string":
      return "";
    case "integer":
    case "number":
      return 0;
    case "boolean":
      return false;
    case "object":
      return {};
    case "array":
      return [];
    default:
      return "";
  }
}

export function SchemaForm({ schema, value, onChange, disabled = false }: Props) {
  const fields = useMemo(() => (schema ? jsonSchemaToFields(schema) : []), [schema]);

  if (!schema || fields.length === 0) {
    return (
      <p className="text-[10px] text-gray-400">
        {schema ? "no fields in schema" : "no schema"}
      </p>
    );
  }

  return (
    <div className="space-y-2">
      {fields.map((f) => (
        <FieldRow
          key={f.name}
          field={f}
          value={value[f.name]}
          onChange={(v) => onChange({ ...value, [f.name]: v })}
          disabled={disabled}
        />
      ))}
    </div>
  );
}

function FieldRow({
  field,
  value,
  onChange,
  disabled,
}: {
  field: JsonSchemaField;
  value: SchemaValue | undefined;
  onChange(v: SchemaValue): void;
  disabled: boolean;
}) {
  const s = field.schema;
  const label = field.name + (field.required ? " *" : "");
  const effective = value === undefined ? defaultForSchema(s) : value;

  let input: React.ReactNode;
  switch (s.type) {
    case "integer":
    case "number":
      input = (
        <input
          aria-label={field.name}
          type="number"
          value={String(effective ?? 0)}
          disabled={disabled}
          onChange={(e) => {
            const n = Number(e.target.value);
            const out = Number.isFinite(n) ? n : 0;
            onChange(s.type === "integer" ? Math.trunc(out) : out);
          }}
          className="flex-1 px-2 py-1 text-xs rounded border border-gray-200 dark:border-neutral-700 bg-white dark:bg-neutral-900 font-mono"
        />
      );
      break;
    case "boolean":
      input = (
        <input
          aria-label={field.name}
          type="checkbox"
          checked={Boolean(effective)}
          disabled={disabled}
          onChange={(e) => onChange(e.target.checked)}
          className="w-4 h-4"
        />
      );
      break;
    case "object": {
      const objValue: SchemaObject =
        effective && typeof effective === "object" && !Array.isArray(effective)
          ? (effective as SchemaObject)
          : {};
      const subFields = s.properties ? jsonSchemaToFields(s) : [];
      input = (
        <div className="flex-1 border border-gray-200 dark:border-neutral-700 rounded p-2 space-y-1">
          {subFields.length === 0 ? (
            <p className="text-[10px] text-gray-400">no nested fields</p>
          ) : (
            subFields.map((sub) => (
              <FieldRow
                key={sub.name}
                field={sub}
                value={objValue[sub.name]}
                onChange={(v) => onChange({ ...objValue, [sub.name]: v })}
                disabled={disabled}
              />
            ))
          )}
        </div>
      );
      break;
    }
    default:
      input = (
        <input
          aria-label={field.name}
          type="text"
          value={String(effective ?? "")}
          disabled={disabled}
          onChange={(e) => onChange(e.target.value)}
          className="flex-1 px-2 py-1 text-xs rounded border border-gray-200 dark:border-neutral-700 bg-white dark:bg-neutral-900 font-mono"
        />
      );
  }

  return (
    <div className="flex items-start gap-3 py-1">
      <label className="w-40 text-xs text-gray-600 dark:text-neutral-400 pt-1">
        {label}
      </label>
      <div className="flex-1">{input}</div>
    </div>
  );
}

export function schemaObjectToFormValue(fields: JsonSchemaField[]): SchemaObject {
  const out: SchemaObject = {};
  for (const f of fields) {
    out[f.name] = defaultForSchema(f.schema);
  }
  return out;
}