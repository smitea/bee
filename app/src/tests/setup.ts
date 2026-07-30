import "@testing-library/jest-dom/vitest";
import { afterEach, vi } from "vitest";
import { cleanup } from "@testing-library/react";
import * as React from "react";

class ResizeObserverMock {
  observe() {}
  unobserve() {}
  disconnect() {}
}

if (typeof globalThis.ResizeObserver === "undefined") {
  globalThis.ResizeObserver = ResizeObserverMock as unknown as typeof ResizeObserver;
}

if (typeof globalThis.matchMedia === "undefined") {
  globalThis.matchMedia = ((query: string) => ({
    matches: false,
    media: query,
    onchange: null,
    addListener: () => {},
    removeListener: () => {},
    addEventListener: () => {},
    removeEventListener: () => {},
    dispatchEvent: () => false,
  })) as unknown as typeof window.matchMedia;
}

if (typeof globalThis.URL.createObjectURL === "undefined") {
  globalThis.URL.createObjectURL = (() => "blob:mock") as typeof URL.createObjectURL;
  globalThis.URL.revokeObjectURL = (() => undefined) as typeof URL.revokeObjectURL;
}

type RfMockNode = {
  id: string;
  type?: string;
  data: unknown;
  position?: { x: number; y: number };
};

type RfMockEdge = {
  id: string;
  type?: string;
  source: string;
  target: string;
  data?: unknown;
  markerEnd?: unknown;
};

const ReactFlowStub = (props: Record<string, unknown>) => {
  const { children, nodes, edges, nodeTypes, edgeTypes } = props;
  const items: React.ReactNode[] = [];
  const nt = (nodeTypes ?? {}) as Record<
    string,
    React.ComponentType<{ id: string; data: unknown }>
  >;
  const et = (edgeTypes ?? {}) as Record<
    string,
    React.ComponentType<Record<string, unknown>>
  >;
  if (Array.isArray(nodes)) {
    for (const n of nodes as RfMockNode[]) {
      const C = nt[n.type ?? "default"];
      if (C) {
        items.push(
          React.createElement(
            "div",
            {
              key: `rf-node-${n.id}`,
              "data-testid": `rf-node-${n.id}`,
              "data-node-id": n.id,
              "data-node-type": n.type ?? "default",
              "data-node-x": String(n.position?.x ?? 0),
              "data-node-y": String(n.position?.y ?? 0),
            },
            React.createElement(C, { id: n.id, data: n.data }),
          ),
        );
      }
    }
  }
  if (Array.isArray(edges)) {
    for (const e of edges as RfMockEdge[]) {
      const E = et[e.type ?? "default"];
      if (E) {
        const edgeProps = {
          id: e.id,
          source: e.source,
          target: e.target,
          data: e.data,
          sourceX: 100,
          sourceY: 100,
          targetX: 200,
          targetY: 200,
          markerEnd: e.markerEnd,
        };
        items.push(
          React.createElement(
            "svg",
            {
              key: `rf-edge-${e.id}`,
              "data-testid": `rf-edge-${e.id}`,
              "data-edge-source": e.source,
              "data-edge-target": e.target,
              "data-edge-type": e.type ?? "default",
              "data-edge-data": JSON.stringify(e.data ?? {}),
            },
            React.createElement(E, edgeProps),
          ),
        );
      }
    }
  }
  return React.createElement(
    "div",
    { "data-testid": "xyflow-reactflow" },
    children as React.ReactNode,
    ...items,
  );
};

const BaseEdgeStub = (props: {
  path?: string;
  markerEnd?: unknown;
  style?: React.CSSProperties;
}) => {
  const marker = props.markerEnd as { color?: string; type?: string } | undefined;
  const markerColor =
    typeof marker === "object" && marker && "color" in marker
      ? String(marker.color ?? "")
      : "";
  return React.createElement("path", {
    "data-testid": "xyflow-baseedge",
    "data-path": props.path ?? "",
    "data-stroke": props.style?.stroke ?? "",
    "data-dasharray": String(props.style?.strokeDasharray ?? ""),
    "data-marker-color": markerColor,
  });
};

vi.mock("@xyflow/react", () => {
  return {
    ReactFlow: ReactFlowStub,
    Background: ({ children }: { children?: React.ReactNode }) =>
      React.createElement(
        "div",
        { "data-testid": "xyflow-background" },
        children,
      ),
    Controls: () =>
      React.createElement("div", { "data-testid": "xyflow-controls" }),
    MiniMap: () =>
      React.createElement("div", { "data-testid": "xyflow-minimap" }),
    BaseEdge: BaseEdgeStub,
    Handle: () => null,
    EdgeLabelRenderer: ({ children }: { children?: React.ReactNode }) =>
      React.createElement(React.Fragment, null, children),
    Panel: ({ children }: { children?: React.ReactNode }) =>
      React.createElement("div", { "data-testid": "xyflow-panel" }, children),
    ViewportPortal: ({ children }: { children?: React.ReactNode }) =>
      React.createElement(React.Fragment, null, children),
    Position: { Top: "top", Bottom: "bottom", Left: "left", Right: "right" },
    MarkerType: { Arrow: "arrow", ArrowClosed: "arrowclosed" },
    ConnectionLineType: {
      Bezier: "bezier",
      Straight: "straight",
      Step: "step",
      SmoothStep: "smoothstep",
      SimpleBezier: "simplebezier",
    },
    ConnectionMode: { Strict: "strict", Loose: "loose" },
    useNodesState: <T,>(initial: T[]) => [initial as T[], () => {}, () => {}],
    useEdgesState: <T,>(initial: T[]) => [initial as T[], () => {}, () => {}],
    useReactFlow: () => ({
      fitView: () => {},
      zoomIn: () => {},
      zoomOut: () => {},
      setCenter: () => {},
    }),
    addEdge: vi.fn(),
    applyEdgeChanges: vi.fn(),
    applyNodeChanges: vi.fn(),
    reconnectEdge: vi.fn(),
  };
});

afterEach(() => cleanup());