import { useState } from "react";

export interface InputRef {
  datasource: string;
  method: string;
  args: Record<string, string | number | boolean>;
  output: string;
}

export interface HandlerRef {
  id: string;
  name: string;
  params: Record<string, string | number | boolean>;
  upstream: string[];
}

export interface OutputRef {
  adapter: string;
  method: string;
  args: Record<string, string | number | boolean>;
  upstream: string;
}

export interface CrossPipelineRef {
  upstreamPipelineName: string;
  upstreamPhaseId: string;
  downstreamPhaseId: string;
}

export interface PipelineDefinition {
  id: number;
  name: string;
  input: InputRef;
  handlers: HandlerRef[];
  output: OutputRef;
  crossPipelineRefs: CrossPipelineRef[];
}

export function parsePipeline(raw: {
  id: number;
  name: string;
  dag_json: string;
  updated_at: number;
}): PipelineDefinition {
  let body: Partial<PipelineDefinition> = {};
  try {
    const parsed: unknown = JSON.parse(raw.dag_json);
    if (parsed && typeof parsed === "object") body = parsed as Partial<PipelineDefinition>;
  } catch {
    body = {};
  }
  const handlers = Array.isArray(body.handlers) ? body.handlers : [];
  return {
    id: raw.id,
    name: raw.name,
    input: body.input ?? defaultInput(),
    handlers: handlers as HandlerRef[],
    output: body.output ?? defaultOutput(),
    crossPipelineRefs: Array.isArray(body.crossPipelineRefs)
      ? (body.crossPipelineRefs as CrossPipelineRef[])
      : [],
  };
}

function defaultInput(): InputRef {
  return { datasource: "(none)", method: "subscribe", args: {}, output: "in" };
}

function defaultOutput(): OutputRef {
  return { adapter: "(none)", method: "emit", args: {}, upstream: "in" };
}

interface PipelineGraphProps {
  pipeline: PipelineDefinition;
  onSelectInput(): void;
  onSelectOutput(): void;
  onSelectHandler(id: string): void;
  onSelectCrossPipelineRef(ref: CrossPipelineRef): void;
}

export function PipelineGraph({
  pipeline,
  onSelectInput,
  onSelectOutput,
  onSelectHandler,
  onSelectCrossPipelineRef,
}: PipelineGraphProps) {
  const [hovered, setHovered] = useState<string | null>(null);
  const refsByHandler = new Map<string, CrossPipelineRef[]>();
  for (const ref of pipeline.crossPipelineRefs) {
    const list = refsByHandler.get(ref.downstreamPhaseId) ?? [];
    list.push(ref);
    refsByHandler.set(ref.downstreamPhaseId, list);
  }

  return (
    <div className="flex gap-6">
      <ol
        aria-label="pipeline graph"
        className="flex-1 flex flex-col gap-3 max-w-md"
      >
        <NodeRow>
          <button
            aria-label="input node"
            type="button"
            onClick={onSelectInput}
            className="w-full text-left px-3 py-2 rounded-md border border-accent-blue/40 bg-accent-blue/5 hover:bg-accent-blue/10"
          >
            <div className="text-[10px] uppercase tracking-wider text-accent-blue font-semibold">
              Input
            </div>
            <div className="text-sm font-mono">
              {pipeline.input.datasource}.{pipeline.input.method}
            </div>
            <div className="text-[10px] text-gray-500 dark:text-neutral-400 font-mono">
              → {pipeline.input.output}
            </div>
          </button>
        </NodeRow>

        {pipeline.handlers.map((h) => {
          const refs = refsByHandler.get(h.id) ?? [];
          return (
            <NodeRow key={h.id}>
              <div className="relative">
                <button
                  type="button"
                  onClick={() => onSelectHandler(h.id)}
                  onMouseEnter={() => setHovered(h.id)}
                  onMouseLeave={() => setHovered((cur) => (cur === h.id ? null : cur))}
                  className="w-full text-left px-3 py-2 rounded-md border border-gray-200 dark:border-neutral-700 bg-white dark:bg-neutral-800 hover:bg-gray-50 dark:hover:bg-neutral-700"
                >
                  <div className="text-[10px] uppercase tracking-wider text-gray-500 dark:text-neutral-400 font-semibold">
                    Handler · {h.id}
                  </div>
                  <div className="text-sm font-mono">{h.name}</div>
                </button>
                {hovered === h.id && (
                  <div
                    role="tooltip"
                    className="absolute left-full ml-3 top-0 z-10 min-w-[12rem] rounded-md border border-gray-200 dark:border-neutral-700 bg-white dark:bg-neutral-800 p-2 shadow-md text-[11px] font-mono"
                  >
                    <div className="font-semibold">{h.name}</div>
                    {Object.entries(h.params).map(([k, v]) => (
                      <div key={k} className="text-gray-600 dark:text-neutral-300">
                        {k}: {String(v)}
                      </div>
                    ))}
                  </div>
                )}
                {refs.length > 0 && (
                  <div className="absolute -right-32 top-0 flex flex-col gap-1">
                    {refs.map((r, i) => (
                      <button
                        key={i}
                        aria-label="cross-pipeline edge"
                        type="button"
                        onClick={() => onSelectCrossPipelineRef(r)}
                        className="flex items-center gap-1 text-[10px] px-1.5 py-0.5 rounded border border-accent-orange/40 bg-accent-orange/10 text-accent-orange hover:bg-accent-orange/20"
                      >
                        <span aria-hidden="true">→</span>
                        <span>{r.upstreamPipelineName}</span>
                      </button>
                    ))}
                  </div>
                )}
              </div>
            </NodeRow>
          );
        })}

        <NodeRow>
          <button
            aria-label="output node"
            type="button"
            onClick={onSelectOutput}
            className="w-full text-left px-3 py-2 rounded-md border border-accent-green/40 bg-accent-green/5 hover:bg-accent-green/10"
          >
            <div className="text-[10px] uppercase tracking-wider text-accent-green font-semibold">
              Output
            </div>
            <div className="text-sm font-mono">
              {pipeline.output.adapter}.{pipeline.output.method}
            </div>
          </button>
        </NodeRow>
      </ol>
    </div>
  );
}

function NodeRow({ children }: { children: React.ReactNode }) {
  return <li className="list-none">{children}</li>;
}
