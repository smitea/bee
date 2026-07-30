import { describe, it, expect, vi } from "vitest";
import { fireEvent, render, screen } from "@testing-library/react";

import {
  ClusterTopology,
  type TopologyNode,
  statusOf,
  statusColor,
} from "../../components/ClusterTopology";

const nodes: TopologyNode[] = [
  { id: 1, role: "Leader", addr: "127.0.0.1:9001", term: 4, lag: 0 },
  { id: 2, role: "Follower", addr: "127.0.0.1:9002", term: 4, lag: 1 },
  { id: 3, role: "Follower", addr: "127.0.0.1:9003", term: 4, lag: 2 },
];

function pxFromStyle(style: string, prop: string): number {
  const re = new RegExp(`${prop}\\s*:\\s*(\\d+)\\s*px`, "i");
  const m = style.match(re);
  return m ? parseInt(m[1] ?? "0", 10) : 0;
}

function bgFromStyle(style: string): string {
  const m = style.match(/background\s*:\s*([^;]+)/i);
  return (m?.[1] ?? "").trim().toLowerCase();
}

function borderFromStyle(style: string): string {
  const m = style.match(/border\s*:\s*([^;]+)/i);
  return (m?.[1] ?? "").trim().toLowerCase();
}

describe("<ClusterTopology>", () => {
  it("renders one node group per cluster node", () => {
    render(<ClusterTopology nodes={nodes} leaderId={1} />);
    expect(screen.getByTestId("topology-node-1")).toBeInTheDocument();
    expect(screen.getByTestId("topology-node-2")).toBeInTheDocument();
    expect(screen.getByTestId("topology-node-3")).toBeInTheDocument();
  });

  it("marks the leader node with the leader role", () => {
    render(<ClusterTopology nodes={nodes} leaderId={1} />);
    expect(screen.getByTestId("topology-node-1").getAttribute("data-role")).toBe("leader");
    expect(screen.getByTestId("topology-node-2").getAttribute("data-role")).toBe("follower");
  });

  it("shows the empty state when no nodes are provided", () => {
    render(<ClusterTopology nodes={[]} leaderId={null} onSelectCluster={() => {}} />);
    expect(screen.getByTestId("cluster-topology-empty")).toBeInTheDocument();
    expect(screen.getByText(/No Bee cluster reachable/)).toBeInTheDocument();
  });

  it("shows the tooltip on hover", () => {
    render(<ClusterTopology nodes={nodes} leaderId={1} />);
    fireEvent.mouseEnter(screen.getByTestId("topology-node-2"));
    expect(screen.getByTestId("topology-tooltip")).toBeInTheDocument();
    expect(screen.getByTestId("topology-tooltip").textContent).toMatch(/node #2/);
    expect(screen.getByTestId("topology-tooltip").textContent).toMatch(/127.0.0.1:9002/);
  });

  it("hides the tooltip when mouse leaves", () => {
    render(<ClusterTopology nodes={nodes} leaderId={1} />);
    fireEvent.mouseEnter(screen.getByTestId("topology-node-2"));
    fireEvent.mouseLeave(screen.getByTestId("topology-node-2"));
    expect(screen.queryByTestId("topology-tooltip")).toBeNull();
  });

  it("shows an error tone for a down follower", () => {
    const down: TopologyNode[] = [
      { id: 1, role: "Leader", addr: "127.0.0.1:9001", term: 4, lag: 0 },
      { id: 2, role: "Follower", addr: "127.0.0.1:9002", term: 4, lag: 1, error: "refused" },
    ];
    render(<ClusterTopology nodes={down} leaderId={1} />);
    const node = screen.getByTestId("topology-node-2");
    expect(node.textContent).toMatch(/×/);
    fireEvent.mouseEnter(node);
    expect(screen.getByTestId("topology-tooltip").textContent).toMatch(/refused/);
  });

  it("invokes onSelectCluster when the empty state button is clicked", () => {
    const onSelect = vi.fn();
    render(<ClusterTopology nodes={[]} leaderId={null} onSelectCluster={onSelect} />);
    fireEvent.click(screen.getByRole("button", { name: /select cluster/i }));
    expect(onSelect).toHaveBeenCalled();
  });

  it("renders exactly one react-flow node per cluster node", () => {
    render(<ClusterTopology nodes={nodes} leaderId={1} />);
    expect(screen.getByTestId("rf-node-n-1")).toBeInTheDocument();
    expect(screen.getByTestId("rf-node-n-2")).toBeInTheDocument();
    expect(screen.getByTestId("rf-node-n-3")).toBeInTheDocument();
    expect(screen.getAllByTestId(/^rf-node-n-/).length).toBe(3);
  });

  it("renders one edge per follower in the leader-follower topology", () => {
    render(<ClusterTopology nodes={nodes} leaderId={1} />);
    expect(screen.getByTestId("rf-edge-e-1-2")).toBeInTheDocument();
    expect(screen.getByTestId("rf-edge-e-1-3")).toBeInTheDocument();
    expect(screen.getAllByTestId(/^rf-edge-e-/).length).toBe(2);
  });

  it("applies green stroke and solid style for a healthy edge", () => {
    render(<ClusterTopology nodes={nodes} leaderId={1} />);
    const edge = screen.getByTestId("rf-edge-e-1-2").querySelector("path")!;
    expect(edge.getAttribute("data-stroke")).toBe("#22c55e");
    expect(edge.getAttribute("data-dasharray")).toBe("");
    expect(edge.getAttribute("data-marker-color")).toBe("#22c55e");
  });

  it("applies amber stroke for a candidate-follower edge", () => {
    const cand: TopologyNode[] = [
      { id: 1, role: "Leader", addr: "127.0.0.1:9001", term: 4, lag: 0 },
      { id: 2, role: "Candidate", addr: "127.0.0.1:9002", term: 4, lag: 0 },
    ];
    render(<ClusterTopology nodes={cand} leaderId={1} />);
    const edge = screen.getByTestId("rf-edge-e-1-2").querySelector("path")!;
    expect(edge.getAttribute("data-stroke")).toBe("#f59e0b");
    expect(edge.getAttribute("data-marker-color")).toBe("#f59e0b");
  });

  it("applies red dashed stroke for a down-follower edge", () => {
    const down: TopologyNode[] = [
      { id: 1, role: "Leader", addr: "127.0.0.1:9001", term: 4, lag: 0 },
      { id: 2, role: "Follower", addr: "127.0.0.1:9002", term: 4, lag: 1, error: "refused" },
    ];
    render(<ClusterTopology nodes={down} leaderId={1} />);
    const edge = screen.getByTestId("rf-edge-e-1-2").querySelector("path")!;
    expect(edge.getAttribute("data-stroke")).toBe("#ef4444");
    expect(edge.getAttribute("data-dasharray")).toMatch(/4/);
    expect(edge.getAttribute("data-marker-color")).toBe("#ef4444");
  });

  it("renders the leader node larger than followers", () => {
    render(<ClusterTopology nodes={nodes} leaderId={1} />);
    const leader = screen.getByTestId("topology-node-1");
    const follower = screen.getByTestId("topology-node-2");
    const leaderStyle = leader.getAttribute("style") ?? "";
    const followerStyle = follower.getAttribute("style") ?? "";
    const leaderW = pxFromStyle(leaderStyle, "width");
    const leaderH = pxFromStyle(leaderStyle, "height");
    const followerW = pxFromStyle(followerStyle, "width");
    const followerH = pxFromStyle(followerStyle, "height");
    expect(leaderW).toBeGreaterThan(followerW);
    expect(leaderH).toBeGreaterThan(followerH);
  });

  it("renders the leader node with an amber border and cream fill", () => {
    render(<ClusterTopology nodes={nodes} leaderId={1} />);
    const leader = screen.getByTestId("topology-node-1");
    const borderEl = leader.querySelector("div")!;
    const style = borderFromStyle(borderEl.getAttribute("style") ?? "");
    expect(style).toMatch(/amber|245,\s*158,\s*11/);
    const bg = bgFromStyle(borderEl.getAttribute("style") ?? "");
    expect(bg).toMatch(/255,\s*251,\s*235/);
  });

  it("renders status dots with green for healthy leader and followers", () => {
    render(<ClusterTopology nodes={nodes} leaderId={1} />);
    const leaderDot = screen.getByTestId("topology-status-1");
    const followerDot = screen.getByTestId("topology-status-2");
    expect(bgFromStyle(leaderDot.getAttribute("style") ?? "")).toMatch(
      /34,\s*197,\s*94/,
    );
    expect(bgFromStyle(followerDot.getAttribute("style") ?? "")).toMatch(
      /34,\s*197,\s*94/,
    );
  });

  it("renders the candidate status dot in amber", () => {
    const cand: TopologyNode[] = [
      { id: 1, role: "Leader", addr: "127.0.0.1:9001", term: 4, lag: 0 },
      { id: 2, role: "Candidate", addr: "127.0.0.1:9002", term: 4, lag: 0 },
    ];
    render(<ClusterTopology nodes={cand} leaderId={1} />);
    const dot = screen.getByTestId("topology-status-2");
    expect(bgFromStyle(dot.getAttribute("style") ?? "")).toMatch(
      /245,\s*158,\s*11/,
    );
  });

  it("renders the down status dot in red", () => {
    const down: TopologyNode[] = [
      { id: 1, role: "Leader", addr: "127.0.0.1:9001", term: 4, lag: 0 },
      { id: 2, role: "Follower", addr: "127.0.0.1:9002", term: 4, lag: 1, error: "refused" },
    ];
    render(<ClusterTopology nodes={down} leaderId={1} />);
    const dot = screen.getByTestId("topology-status-2");
    expect(bgFromStyle(dot.getAttribute("style") ?? "")).toMatch(
      /239,\s*68,\s*68/,
    );
  });

  it("falls back to gray for an unknown status color", () => {
    const unknown = {
      id: 7,
      role: "Follower" as const,
      addr: "127.0.0.1:9007",
      term: 1,
      lag: 0,
    };
    expect(statusOf(unknown)).toBe("healthy");
    expect(statusColor("unknown")).toBe("#9ca3af");
  });

  it("shows the empty state ghost topology when no nodes are reachable", () => {
    const { container } = render(
      <ClusterTopology nodes={[]} leaderId={null} onSelectCluster={() => {}} />,
    );
    expect(screen.getByTestId("cluster-topology-empty")).toBeInTheDocument();
    expect(container.querySelector('[data-testid="rf-node-n-1"]')).toBeTruthy();
    expect(container.querySelector('[data-testid="rf-node-n-2"]')).toBeTruthy();
  });

  it("renders all five nodes when AdminServer reports a 5-node cluster", () => {
    const fiveNodes: TopologyNode[] = [
      { id: 1, role: "Leader", addr: "127.0.0.1:9001", term: 5, lag: 0 },
      { id: 2, role: "Follower", addr: "127.0.0.1:9002", term: 5, lag: 0 },
      { id: 3, role: "Follower", addr: "127.0.0.1:9003", term: 5, lag: 1 },
      { id: 4, role: "Candidate", addr: "127.0.0.1:9004", term: 5, lag: 0 },
      { id: 5, role: "Follower", addr: "127.0.0.1:9005", term: 5, lag: 2 },
    ];
    render(<ClusterTopology nodes={fiveNodes} leaderId={1} />);
    for (const id of [1, 2, 3, 4, 5]) {
      expect(screen.getByTestId(`topology-node-${id}`)).toBeInTheDocument();
      expect(screen.getByTestId(`topology-status-${id}`)).toBeInTheDocument();
      expect(screen.getByTestId(`rf-node-n-${id}`)).toBeInTheDocument();
    }
    expect(screen.getAllByTestId(/^topology-node-/).length).toBe(5);
    expect(screen.getAllByTestId(/^rf-node-n-/).length).toBe(5);
    expect(screen.getByTestId("topology-node-1").getAttribute("data-role")).toBe("leader");
    expect(screen.getByTestId("topology-node-4").getAttribute("data-role")).toBe("candidate");
    for (const id of [2, 3, 5]) {
      expect(
        screen.getByTestId(`topology-node-${id}`).getAttribute("data-role"),
      ).toBe("follower");
    }
  });

  it("renders exactly one react-flow edge from leader to each follower in a 5-node cluster", () => {
    const fiveNodes: TopologyNode[] = [
      { id: 1, role: "Leader", addr: "127.0.0.1:9001", term: 5, lag: 0 },
      { id: 2, role: "Follower", addr: "127.0.0.1:9002", term: 5, lag: 0 },
      { id: 3, role: "Follower", addr: "127.0.0.1:9003", term: 5, lag: 0 },
      { id: 4, role: "Follower", addr: "127.0.0.1:9004", term: 5, lag: 0 },
      { id: 5, role: "Follower", addr: "127.0.0.1:9005", term: 5, lag: 0 },
    ];
    render(<ClusterTopology nodes={fiveNodes} leaderId={1} />);
    expect(screen.getByTestId("rf-edge-e-1-2")).toBeInTheDocument();
    expect(screen.getByTestId("rf-edge-e-1-3")).toBeInTheDocument();
    expect(screen.getByTestId("rf-edge-e-1-4")).toBeInTheDocument();
    expect(screen.getByTestId("rf-edge-e-1-5")).toBeInTheDocument();
    expect(screen.getAllByTestId(/^rf-edge-e-/).length).toBe(4);
  });

  it("maps a down follower to the down status in a multi-node cluster", () => {
    const mixed: TopologyNode[] = [
      { id: 1, role: "Leader", addr: "127.0.0.1:9001", term: 5, lag: 0 },
      { id: 2, role: "Follower", addr: "127.0.0.1:9002", term: 5, lag: 0 },
      { id: 3, role: "Follower", addr: "127.0.0.1:9003", term: 5, lag: 0, error: "refused" },
      { id: 4, role: "Follower", addr: "127.0.0.1:9004", term: 5, lag: 0 },
      { id: 5, role: "Follower", addr: "127.0.0.1:9005", term: 5, lag: 0 },
    ];
    render(<ClusterTopology nodes={mixed} leaderId={1} />);
    expect(
      screen.getByTestId("topology-status-3").getAttribute("data-status-dot"),
    ).toBe("down");
    expect(
      screen.getByTestId("topology-status-1").getAttribute("data-status-dot"),
    ).toBe("healthy");
    expect(
      screen.getByTestId("topology-status-4").getAttribute("data-status-dot"),
    ).toBe("healthy");
  });

  it("handles the ClusterDashboard-style mapping where every node has an id, role, commit_index, log_length", () => {
    const fromAdmin: {
      id: number;
      role: string;
      commit_index: number;
      log_length: number;
    }[] = [
      { id: 1, role: "Leader", commit_index: 100, log_length: 100 },
      { id: 2, role: "Follower", commit_index: 100, log_length: 100 },
      { id: 3, role: "Follower", commit_index: 100, log_length: 98 },
      { id: 4, role: "Candidate", commit_index: 99, log_length: 99 },
      { id: 5, role: "Follower", commit_index: 100, log_length: 95 },
    ];
    const mapped: TopologyNode[] = fromAdmin.map((n) => ({
      id: n.id,
      role: n.role as TopologyNode["role"],
      addr: "127.0.0.1:9999",
      term: 7,
      lag: n.commit_index - n.log_length,
      error:
        n.role === "Follower" && n.log_length < n.commit_index
          ? `lag ${n.commit_index - n.log_length}`
          : null,
    }));
    render(<ClusterTopology nodes={mapped} leaderId={1} />);
    for (const n of fromAdmin) {
      expect(screen.getByTestId(`topology-node-${n.id}`)).toBeInTheDocument();
      expect(
        screen.getByTestId(`topology-node-${n.id}`).getAttribute("data-role"),
      ).toBe(n.role.toLowerCase());
    }
    expect(screen.getByTestId("topology-status-4").getAttribute("data-status-dot")).toBe(
      "candidate",
    );
  });
});