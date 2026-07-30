import { describe, it, expect, vi } from "vitest";
import { fireEvent, render, screen } from "@testing-library/react";

import { ClusterTopology, type TopologyNode } from "../../components/ClusterTopology";

const nodes: TopologyNode[] = [
  { id: 1, role: "Leader", addr: "127.0.0.1:9001", term: 4, lag: 0 },
  { id: 2, role: "Follower", addr: "127.0.0.1:9002", term: 4, lag: 1 },
  { id: 3, role: "Follower", addr: "127.0.0.1:9003", term: 4, lag: 2 },
];

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
});
