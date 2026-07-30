import { useEffect, useMemo, useState } from "react";
import {
  Background,
  BaseEdge,
  Handle,
  MarkerType,
  Position,
  ReactFlow,
  useEdgesState,
  useNodesState,
  type Edge,
  type EdgeProps,
  type Node,
  type NodeProps,
} from "@xyflow/react";
import "@xyflow/react/dist/style.css";

export interface TopologyNode {
  id: number;
  role: "Leader" | "Follower" | "Candidate";
  addr: string;
  term: number;
  lag: number;
  error?: string | null;
}

interface Props {
  nodes: TopologyNode[];
  leaderId: number | null;
  onSelectCluster?(): void;
}

export type TopologyStatus = "healthy" | "candidate" | "down" | "unknown";

export function statusOf(
  n: Pick<TopologyNode, "role" | "error">,
): TopologyStatus {
  if (n.error) return "down";
  if (n.role === "Leader") return "healthy";
  if (n.role === "Candidate") return "candidate";
  if (n.role === "Follower") return "healthy";
  return "unknown";
}

export function statusColor(s: TopologyStatus): string {
  switch (s) {
    case "healthy":
      return "#22c55e";
    case "candidate":
      return "#f59e0b";
    case "down":
      return "#ef4444";
    default:
      return "#9ca3af";
  }
}

const CENTER = { x: 200, y: 160 };
const RADIUS = 120;
const FOLLOW_ANGLES = [0, 60, 120, 180, 240, 300];
const LEADER_W = 91;
const LEADER_H = 52;
const FOLLOW_W = 26;
const FOLLOW_H = 30;
const EMPTY_VIEW_W = 400;
const EMPTY_VIEW_H = 320;

type ServerNodeData = {
  topology: TopologyNode;
  isLeader: boolean;
  status: TopologyStatus;
  onHover?: (n: TopologyNode | null) => void;
};
type RfNode = Node<ServerNodeData, "server">;
type RfEdge = Edge<{ status: TopologyStatus }, "orthogonal">;

export function computeLayout(
  nodes: TopologyNode[],
  leaderId: number | null,
): { rfNodes: RfNode[]; rfEdges: RfEdge[] } {
  const leader =
    nodes.find((n) => n.id === leaderId) ??
    nodes.find((n) => n.role === "Leader") ??
    null;
  const followers = nodes.filter((n) => n !== leader);
  const rfNodes: RfNode[] = [];
  if (leader) {
    rfNodes.push({
      id: `n-${leader.id}`,
      type: "server",
      position: {
        x: CENTER.x - LEADER_W / 2,
        y: CENTER.y - LEADER_H / 2,
      },
      data: {
        topology: leader,
        isLeader: true,
        status: statusOf(leader),
      },
    });
  }
  followers.forEach((f, i) => {
    const angle = FOLLOW_ANGLES[i % FOLLOW_ANGLES.length] ?? 0;
    const x =
      CENTER.x + RADIUS * Math.cos((angle * Math.PI) / 180) - FOLLOW_W / 2;
    const y =
      CENTER.y + RADIUS * Math.sin((angle * Math.PI) / 180) - FOLLOW_H / 2;
    rfNodes.push({
      id: `n-${f.id}`,
      type: "server",
      position: { x, y },
      data: {
        topology: f,
        isLeader: false,
        status: statusOf(f),
      },
    });
  });
  const rfEdges: RfEdge[] = [];
  if (leader) {
    followers.forEach((f) => {
      const status = statusOf(f);
      rfEdges.push({
        id: `e-${leader.id}-${f.id}`,
        source: `n-${leader.id}`,
        target: `n-${f.id}`,
        type: "orthogonal",
        data: { status },
        markerEnd: {
          type: MarkerType.ArrowClosed,
          color: statusColor(status),
          width: 14,
          height: 14,
        },
      });
    });
  }
  return { rfNodes, rfEdges };
}

function ServerNode({ data }: NodeProps<RfNode>) {
  const { topology, isLeader, status, onHover } = data;
  const w = isLeader ? LEADER_W : FOLLOW_W;
  const h = isLeader ? LEADER_H : FOLLOW_H;
  const isDown = status === "down";
  const borderColor = isLeader
    ? "#f59e0b"
    : isDown
      ? "#9ca3af"
      : "#6b7280";
  const fill = isLeader ? "#fffbeb" : isDown ? "#e5e7eb" : "#ffffff";

  return (
    <div
      className="relative"
      style={{ width: w, height: h }}
      data-testid={`topology-node-${topology.id}`}
      data-role={topology.role.toLowerCase()}
      data-status={status}
      onMouseEnter={() => onHover?.(topology)}
      onMouseLeave={() => onHover?.(null)}
    >
      <Handle
        type="target"
        position={Position.Top}
        style={{ opacity: 0, pointerEvents: "none" }}
      />
      <Handle
        type="source"
        position={Position.Bottom}
        style={{ opacity: 0, pointerEvents: "none" }}
      />
      <div
        className="absolute inset-0 rounded"
        style={{
          background: fill,
          border: `${isLeader ? 2 : 1.5}px solid ${borderColor}`,
          opacity: isDown ? 0.5 : 1,
        }}
      >
        {!isLeader && !isDown && (
          <div className="absolute inset-1 flex flex-col gap-[2px]">
            {[0, 1, 2, 3].map((k) => (
              <div key={k} className="h-[1px] bg-gray-400 opacity-60" />
            ))}
          </div>
        )}
        {isLeader && (
          <div className="absolute inset-0 flex items-center justify-center">
            <span className="text-[9px] font-bold tracking-wider text-amber-700">
              LEADER
            </span>
          </div>
        )}
        {isDown && (
          <div className="absolute inset-0 flex items-center justify-center">
            <span className="text-base font-bold leading-none text-red-500">
              ×
            </span>
          </div>
        )}
      </div>
      <div
        className="absolute rounded-full"
        style={{
          width: 10,
          height: 10,
          top: h / 2 - 5,
          left: -14,
          background: statusColor(status),
        }}
        data-testid={`topology-status-${topology.id}`}
        data-status-dot={status}
      />
    </div>
  );
}

const nodeTypes = { server: ServerNode };

function OrthogonalEdge(props: EdgeProps<RfEdge>) {
  const { sourceX, sourceY, targetX, targetY, data, markerEnd } = props;
  const status = data?.status ?? "healthy";
  const color = statusColor(status);
  const midX = (sourceX + targetX) / 2;
  const path = `M ${sourceX} ${sourceY} L ${midX} ${sourceY} L ${midX} ${targetY} L ${targetX} ${targetY}`;
  return (
    <BaseEdge
      path={path}
      markerEnd={markerEnd}
      style={{
        stroke: color,
        strokeWidth: 1.5,
        strokeDasharray: status === "down" ? "4 4" : undefined,
      }}
    />
  );
}

const edgeTypes = { orthogonal: OrthogonalEdge };

function GhostTopology({
  leaderId,
  onSelectCluster,
}: {
  leaderId: number | null;
  onSelectCluster?: () => void;
}) {
  const ghostNodes: TopologyNode[] = useMemo(() => {
    const out: TopologyNode[] = [];
    out.push({
      id: 1,
      role: "Leader",
      addr: "—",
      term: 0,
      lag: 0,
    });
    for (let i = 0; i < 6; i += 1) {
      out.push({
        id: i + 2,
        role: "Follower",
        addr: "—",
        term: 0,
        lag: 0,
      });
    }
    return out;
  }, []);

  const { rfNodes, rfEdges } = useMemo(
    () => computeLayout(ghostNodes, leaderId ?? 1),
    [ghostNodes, leaderId],
  );
  const [nodes] = useNodesState<RfNode>(rfNodes);
  const [edges] = useEdgesState<RfEdge>(rfEdges);

  return (
    <div
      className="flex flex-col items-center justify-center gap-2"
      data-testid="cluster-topology-empty"
    >
      <div
        style={{ width: EMPTY_VIEW_W, height: EMPTY_VIEW_H, opacity: 0.3 }}
      >
        <ReactFlow
          nodes={nodes}
          edges={edges}
          nodeTypes={nodeTypes}
          edgeTypes={edgeTypes}
          fitView
          fitViewOptions={{ padding: 0.2 }}
          nodesDraggable={false}
          nodesConnectable={false}
          elementsSelectable={false}
          zoomOnScroll={false}
          panOnScroll={false}
          panOnDrag={false}
          proOptions={{ hideAttribution: true }}
        >
          <Background gap={20} size={0.5} color="#e5e7eb" />
        </ReactFlow>
      </div>
      <p className="text-xs text-gray-400">No Bee cluster reachable</p>
      {onSelectCluster && (
        <button
          type="button"
          onClick={onSelectCluster}
          className="px-2 py-1 text-[11px] rounded border border-gray-200 dark:border-neutral-700"
        >
          Select cluster
        </button>
      )}
    </div>
  );
}

export function ClusterTopology({ nodes, leaderId, onSelectCluster }: Props) {
  const [hovered, setHovered] = useState<TopologyNode | null>(null);

  const { rfNodes: initialNodes, rfEdges: initialEdges } = useMemo(
    () => computeLayout(nodes, leaderId),
    [nodes, leaderId],
  );

  const [rfNodes, setRfNodes, onNodesChange] = useNodesState<RfNode>(initialNodes);
  const [rfEdges, setRfEdges, onEdgesChange] = useEdgesState<RfEdge>(initialEdges);

  useEffect(() => {
    setRfNodes(initialNodes);
    setRfEdges(initialEdges);
  }, [initialNodes, initialEdges, setRfNodes, setRfEdges]);

  const nodesWithHover = useMemo<RfNode[]>(
    () =>
      rfNodes.map((n) => ({
        ...n,
        data: { ...n.data, onHover: setHovered },
      })),
    [rfNodes],
  );

  if (nodes.length === 0) {
    return (
      <GhostTopology leaderId={leaderId} onSelectCluster={onSelectCluster} />
    );
  }

  return (
    <div
      className="relative w-full"
      data-testid="cluster-topology"
      style={{ width: EMPTY_VIEW_W, height: EMPTY_VIEW_H }}
    >
      <ReactFlow
        nodes={nodesWithHover}
        edges={rfEdges}
        nodeTypes={nodeTypes}
        edgeTypes={edgeTypes}
        onNodesChange={onNodesChange}
        onEdgesChange={onEdgesChange}
        fitView
        fitViewOptions={{ padding: 0.2 }}
        nodesDraggable={false}
        nodesConnectable={false}
        elementsSelectable={false}
        zoomOnScroll={false}
        panOnScroll={false}
        panOnDrag={false}
        proOptions={{ hideAttribution: true }}
      >
        <Background gap={20} size={0.5} color="#e5e7eb" />
      </ReactFlow>
      {hovered && (
        <div
          className="absolute right-2 top-2 rounded-md border border-gray-200 dark:border-neutral-700 bg-white dark:bg-neutral-800 px-2 py-1 text-[10px] shadow"
          data-testid="topology-tooltip"
        >
          <div className="font-mono">node #{hovered.id}</div>
          <div className="text-gray-500">{hovered.addr}</div>
          <div>role: {hovered.role}</div>
          <div>term: {hovered.term}</div>
          <div>lag: {hovered.lag}</div>
          {hovered.error && (
            <div className="text-accent-red">{hovered.error}</div>
          )}
        </div>
      )}
    </div>
  );
}