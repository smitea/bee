import { useEffect, useMemo, useRef, useState } from "react";
import {
  ReactFlow,
  Background,
  Controls,
  MiniMap,
  addEdge,
  applyEdgeChanges,
  applyNodeChanges,
  type Connection,
  type Edge,
  type EdgeChange,
  type Node,
  type NodeChange,
} from "@xyflow/react";
import "@xyflow/react/dist/style.css";

interface DagInput {
  input?: { datasource?: string; method?: string; output?: string };
  handlers?: Array<{
    id: string;
    name: string;
    upstream?: string[];
  }>;
  output?: { adapter?: string; method?: string; upstream?: string };
}

interface Props {
  dag: DagInput;
  onDagChange(next: DagInput): void;
}

function dagToNodesEdges(
  dag: DagInput,
  fallback: string,
): { nodes: Node[]; edges: Edge[] } {
  const nodes: Node[] = [];
  const edges: Edge[] = [];
  const inId = dag.input?.output ?? "in";
  nodes.push({
    id: "input",
    type: "default",
    position: { x: 40, y: 40 },
    data: {
      label: `Input · ${dag.input?.datasource ?? "?"}.${dag.input?.method ?? "?"}`,
    },
    style: { background: "#dbeafe", border: "1px solid #3b82f6", borderRadius: 4 },
  });
  let y = 120;
  for (const h of dag.handlers ?? []) {
    nodes.push({
      id: h.id,
      type: "default",
      position: { x: 240, y },
      data: { label: `Handler · ${h.id}\n${h.name}` },
      style: { background: "#fff", border: "1px solid #9ca3af", borderRadius: 4 },
    });
    for (const up of h.upstream ?? []) {
      edges.push({ id: `${up}->${h.id}`, source: up, target: h.id });
    }
    y += 80;
  }
  const outId = `output-${fallback}`;
  nodes.push({
    id: outId,
    type: "default",
    position: { x: 480, y: 40 },
    data: {
      label: `Output · ${dag.output?.adapter ?? "?"}.${dag.output?.method ?? "?"}`,
    },
    style: { background: "#dcfce7", border: "1px solid #22c55e", borderRadius: 4 },
  });
  const lastUp = dag.output?.upstream ?? inId;
  edges.push({ id: `${lastUp}->${outId}`, source: lastUp, target: outId });
  return { nodes, edges };
}

function edgesToDagUpdate(dag: DagInput, edges: Edge[], fallback: string): DagInput {
  const inId = dag.input?.output ?? "in";
  const outId = `output-${fallback}`;
  const handlers = (dag.handlers ?? []).map((h) => ({ ...h, upstream: [...(h.upstream ?? [])] }));
  const output = { ...(dag.output ?? {}) };
  for (const e of edges) {
    if (e.source === inId) {
      const tgt = handlers.find((h) => h.id === e.target);
      if (tgt && !tgt.upstream.includes(inId)) {
        tgt.upstream.push(inId);
      }
      continue;
    }
    if (e.target === outId) {
      output.upstream = e.source;
      continue;
    }
    const tgt = handlers.find((h) => h.id === e.target);
    if (tgt && !tgt.upstream.includes(e.source)) {
      tgt.upstream.push(e.source);
    }
  }
  return { ...dag, handlers, output };
}

export function PipelineDagPane({ dag, onDagChange }: Props) {
  const initial = useMemo(() => dagToNodesEdges(dag, "1"), [dag]);
  const [nodes, setNodes] = useState<Node[]>(initial.nodes);
  const [edges, setEdges] = useState<Edge[]>(initial.edges);
  const initializedRef = useRef(false);

  useEffect(() => {
    if (!initializedRef.current) {
      initializedRef.current = true;
      return;
    }
    const next = dagToNodesEdges(dag, "1");
    setNodes(next.nodes);
    setEdges(next.edges);
  }, [dag]);

  const onNodesChange = (changes: NodeChange[]) => {
    setNodes((nds) => applyNodeChanges(changes, nds));
  };
  const onEdgesChange = (changes: EdgeChange[]) => {
    setEdges((eds) => applyEdgeChanges(changes, eds));
  };
  const onConnect = (c: Connection) => {
    setEdges((eds) =>
      addEdge({ ...c, id: `${c.source}->${c.target}` }, eds),
    );
  };

  useEffect(() => {
    if (!initializedRef.current) return;
    onDagChange(edgesToDagUpdate(dag, edges, "1"));
  }, [edges]);

  return (
    <ReactFlow
      nodes={nodes}
      edges={edges}
      onNodesChange={onNodesChange}
      onEdgesChange={onEdgesChange}
      onConnect={onConnect}
      fitView
    >
      <Background gap={20} />
      <Controls />
      <MiniMap pannable zoomable />
    </ReactFlow>
  );
}