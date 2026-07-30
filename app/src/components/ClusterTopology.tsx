import { useState } from "react";

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

const VIEW_W = 400;
const VIEW_H = 320;
const CENTER_X = 200;
const CENTER_Y = 160;
const RADIUS = 100;

const FOLLOWS_AT_DEG = [0, 60, 120, 180, 240, 300];

export function ClusterTopology({ nodes, leaderId, onSelectCluster }: Props) {
  const [hovered, setHovered] = useState<TopologyNode | null>(null);

  if (nodes.length === 0) {
    return (
      <div
        className="flex flex-col items-center justify-center gap-2 text-xs text-gray-400"
        data-testid="cluster-topology-empty"
      >
        <svg width={VIEW_W} height={VIEW_H} role="img" aria-label="ghost topology">
          <g opacity={0.3}>
            <rect x={180} y={140} width={40} height={40} stroke="#9aa0a6" strokeWidth={1} fill="none" />
            {FOLLOWS_AT_DEG.map((deg, i) => {
              const x = CENTER_X + RADIUS * Math.cos((deg * Math.PI) / 180) - 10;
              const y = CENTER_Y + RADIUS * Math.sin((deg * Math.PI) / 180) - 12;
              return (
                <rect key={i} x={x} y={y} width={20} height={24} stroke="#9aa0a6" strokeWidth={1} fill="none" />
              );
            })}
          </g>
        </svg>
        <p>No Bee cluster reachable</p>
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

  const leader = nodes.find((n) => n.id === leaderId) ?? nodes.find((n) => n.role === "Leader") ?? null;
  const followers = nodes.filter((n) => n !== leader);

  return (
    <div className="relative w-full" data-testid="cluster-topology">
      <svg width={VIEW_W} height={VIEW_H} role="img" aria-label="cluster topology">
        <defs>
          <marker id="arrow-green" markerWidth={6} markerHeight={6} refX={5} refY={3} orient="auto">
            <path d="M0,0 L6,3 L0,6 z" fill="#22c55e" />
          </marker>
          <marker id="arrow-red" markerWidth={6} markerHeight={6} refX={5} refY={3} orient="auto">
            <path d="M0,0 L6,3 L0,6 z" fill="#ef4444" />
          </marker>
          <pattern id="grid" width={20} height={20} patternUnits="userSpaceOnUse">
            <path d="M 20 0 L 0 0 0 20" fill="none" stroke="#e5e7eb" strokeWidth={0.5} />
          </pattern>
        </defs>
        <rect width={VIEW_W} height={VIEW_H} fill="url(#grid)" />

        {leader && followers.map((f, i) => {
          const angle = FOLLOWS_AT_DEG[i % FOLLOWS_AT_DEG.length] ?? 0;
          const fx = CENTER_X + RADIUS * Math.cos((angle * Math.PI) / 180);
          const fy = CENTER_Y + RADIUS * Math.sin((angle * Math.PI) / 180);
          const midX = (CENTER_X + fx) / 2;
          const path = `M ${CENTER_X} ${CENTER_Y} L ${midX} ${CENTER_Y} L ${midX} ${fy} L ${fx} ${fy}`;
          const isDown = !!f.error;
          return (
            <path
              key={f.id}
              d={path}
              fill="none"
              stroke={isDown ? "#ef4444" : "#22c55e"}
              strokeWidth={1.5}
              strokeDasharray={isDown ? "4 4" : "none"}
              markerEnd={`url(#${isDown ? "arrow-red" : "arrow-green"})`}
            />
          );
        })}

        {leader && (
          <g
            onMouseEnter={() => setHovered(leader)}
            onMouseLeave={() => setHovered(null)}
            data-testid={`topology-node-${leader.id}`}
            data-role="leader"
          >
            <rect
              x={CENTER_X - 35}
              y={CENTER_Y - 20}
              width={70}
              height={40}
              rx={4}
              fill="#fffbeb"
              stroke="#f59e0b"
              strokeWidth={2}
            />
            <circle cx={CENTER_X - 30} cy={CENTER_Y} r={5} fill="#f59e0b" />
            <text
              x={CENTER_X}
              y={CENTER_Y + 4}
              textAnchor="middle"
              fontSize={10}
              fontWeight={700}
              fill="#b45309"
            >
              LEADER
            </text>
          </g>
        )}

        {followers.map((f, i) => {
          const angle = FOLLOWS_AT_DEG[i % FOLLOWS_AT_DEG.length] ?? 0;
          const x = CENTER_X + RADIUS * Math.cos((angle * Math.PI) / 180);
          const y = CENTER_Y + RADIUS * Math.sin((angle * Math.PI) / 180);
          const isDown = !!f.error;
          return (
            <g
              key={f.id}
              onMouseEnter={() => setHovered(f)}
              onMouseLeave={() => setHovered(null)}
              data-testid={`topology-node-${f.id}`}
              data-role={f.role.toLowerCase()}
            >
              <rect
                x={x - 10}
                y={y - 12}
                width={20}
                height={24}
                rx={2}
                fill={isDown ? "#e5e7eb" : "#fff"}
                stroke={isDown ? "#9ca3af" : "#6b7280"}
                strokeWidth={1}
                opacity={isDown ? 0.5 : 1}
              />
              {[0, 1, 2, 3].map((k) => (
                <line
                  key={k}
                  x1={x - 7}
                  y1={y - 8 + k * 4}
                  x2={x + 7}
                  y2={y - 8 + k * 4}
                  stroke="#9ca3af"
                  strokeWidth={0.5}
                />
              ))}
              <circle
                cx={x - 14}
                cy={y}
                r={5}
                fill={isDown ? "#ef4444" : f.role === "Candidate" ? "#f59e0b" : "#22c55e"}
              />
              {isDown && (
                <text
                  x={x}
                  y={y + 4}
                  textAnchor="middle"
                  fontSize={10}
                  fill="#ef4444"
                >
                  ×
                </text>
              )}
            </g>
          );
        })}
      </svg>
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
