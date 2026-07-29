import { describe, it, expect, vi, beforeEach } from "vitest";
import { QueryClient, QueryClientProvider } from "@tanstack/react-query";
import { render, screen, fireEvent, within, waitFor } from "@testing-library/react";

const mocks = vi.hoisted(() => ({
  clusterStatus: vi.fn(),
  listJobs: vi.fn(),
  setAddr: vi.fn(),
  testConnection: vi.fn(),
  ping: vi.fn(),
  rollingRestartApply: vi.fn(),
}));

vi.mock("../../ipc", async () => {
  const actual = await vi.importActual<typeof import("../../ipc")>("../../ipc");
  return {
    ...actual,
    clusterStatus: mocks.clusterStatus,
    listJobs: mocks.listJobs,
    setAddr: mocks.setAddr,
    testConnection: mocks.testConnection,
    ping: mocks.ping,
    rollingRestartApply: mocks.rollingRestartApply,
  };
});

beforeEach(() => {
  vi.resetModules();
  mocks.clusterStatus.mockReset();
  mocks.listJobs.mockReset();
  mocks.setAddr.mockReset();
  mocks.testConnection.mockReset();
  mocks.ping.mockReset();
  mocks.rollingRestartApply.mockReset();

  mocks.clusterStatus.mockResolvedValue({
    nodes: [
      { id: 1, role: "Leader", commit_index: 12, log_length: 18 },
      { id: 2, role: "Follower", commit_index: 12, log_length: 18 },
      { id: 3, role: "Follower", commit_index: 12, log_length: 18 },
    ],
    leader_id: 1,
    term: 4,
    commit_index: 12,
  });
  mocks.listJobs.mockResolvedValue([]);
  mocks.rollingRestartApply.mockResolvedValue({
    nodes: [
      { id: "n1", addr: "127.0.0.1:9001" },
      { id: "n2", addr: "127.0.0.1:9002" },
      { id: "n3", addr: "127.0.0.1:9003" },
    ],
    batch_size: 1,
    health_timeout_ms: 30_000,
  });
});

function withClient(node: React.ReactNode) {
  const client = new QueryClient({
    defaultOptions: { queries: { retry: false, gcTime: 0 } },
  });
  return render(<QueryClientProvider client={client}>{node}</QueryClientProvider>);
}

describe("<ClusterDashboard>", () => {
  it("renders the topology heading and nodes table", async () => {
    const { ClusterDashboard } = await import("../../pages/ClusterDashboard");
    withClient(<ClusterDashboard />);
    expect(await screen.findByTestId("node-row-1")).toBeInTheDocument();
    expect(screen.getByTestId("node-row-2")).toBeInTheDocument();
    expect(within(screen.getByTestId("node-row-1")).getByText("Leader")).toBeInTheDocument();
    expect(screen.getByText(/Topology/)).toBeInTheDocument();
    expect(screen.getByText(/quorum/i)).toBeInTheDocument();
  });

  it("renders the four job-state counters", async () => {
    const { ClusterDashboard } = await import("../../pages/ClusterDashboard");
    withClient(<ClusterDashboard />);
    expect(await screen.findByTestId("pipeline-jobs-queued")).toBeInTheDocument();
    expect(screen.getByTestId("pipeline-jobs-running")).toBeInTheDocument();
    expect(screen.getByTestId("pipeline-jobs-historical")).toBeInTheDocument();
    expect(screen.getByTestId("pipeline-jobs-failed")).toBeInTheDocument();
  });

  it("partitions jobs into counters by lifecycle", async () => {
    mocks.listJobs.mockResolvedValueOnce([
      { job_id: 1, dag_hash: "x", lifecycle: "WaitingForUpstream", mode: "x", task_count: 0, owner_node: 1 },
      { job_id: 2, dag_hash: "y", lifecycle: "Running", mode: "x", task_count: 1, owner_node: 1 },
      { job_id: 3, dag_hash: "z", lifecycle: "Completed", mode: "x", task_count: 1, owner_node: 1 },
      { job_id: 4, dag_hash: "w", lifecycle: "Failed", mode: "x", task_count: 1, owner_node: 1 },
    ]);
    const { ClusterDashboard } = await import("../../pages/ClusterDashboard");
    withClient(<ClusterDashboard />);
    const queued = screen.getByTestId("pipeline-jobs-queued");
    await within(queued).findByText("1");
    await within(screen.getByTestId("pipeline-jobs-running")).findByText("1");
    await within(screen.getByTestId("pipeline-jobs-historical")).findByText("1");
    await within(screen.getByTestId("pipeline-jobs-failed")).findByText("1");
  });

  it("computes longest-running and highest-resource jobs from job_id order", async () => {
    mocks.listJobs.mockResolvedValueOnce([
      { job_id: 10, dag_hash: "a", lifecycle: "Running", mode: "x", task_count: 2, owner_node: 1 },
      { job_id: 7, dag_hash: "b", lifecycle: "WaitingForUpstream", mode: "x", task_count: 9, owner_node: 1 },
      { job_id: 99, dag_hash: "c", lifecycle: "Running", mode: "x", task_count: 1, owner_node: 1 },
    ]);
    const { ClusterDashboard } = await import("../../pages/ClusterDashboard");
    withClient(<ClusterDashboard />);
    const longest = screen.getByTestId("metric-longest");
    await within(longest).findByText(/#10/);
    const resource = screen.getByTestId("metric-resource");
    await within(resource).findByText(/#7/);
  });

  it("shows 'no data' placeholders when no jobs exist", async () => {
    const { ClusterDashboard } = await import("../../pages/ClusterDashboard");
    withClient(<ClusterDashboard />);
    const longest = await screen.findByTestId("metric-longest");
    expect(longest.textContent).toMatch(/no data/);
    expect(screen.getByTestId("metric-resource").textContent).toMatch(/no data/);
  });

  it("renders configuration controls including Rolling restart button", async () => {
    const { ClusterDashboard } = await import("../../pages/ClusterDashboard");
    withClient(<ClusterDashboard />);
    expect(await screen.findByTestId("rolling-restart")).toBeInTheDocument();
    expect(screen.getByText(/active config/i)).toBeInTheDocument();
  });

  it("Rolling restart button calls rolling_restart_apply and shows the plan dialog", async () => {
    const { ClusterDashboard } = await import("../../pages/ClusterDashboard");
    withClient(<ClusterDashboard />);
    const btn = await screen.findByTestId("rolling-restart");
    fireEvent.click(btn);
    await waitFor(() => expect(mocks.rollingRestartApply).toHaveBeenCalled());
    const dialog = await screen.findByTestId("rolling-restart-plan");
    expect(dialog).toBeInTheDocument();
    expect(within(dialog).getByText("n1")).toBeInTheDocument();
    expect(within(dialog).getByText("n2")).toBeInTheDocument();
    expect(within(dialog).getByText("n3")).toBeInTheDocument();
    expect(within(dialog).getByText(/batch=1/)).toBeInTheDocument();
  });

  it("Rolling restart plan dialog can be closed", async () => {
    const { ClusterDashboard } = await import("../../pages/ClusterDashboard");
    withClient(<ClusterDashboard />);
    fireEvent.click(await screen.findByTestId("rolling-restart"));
    const dialog = await screen.findByTestId("rolling-restart-plan");
    const buttons = within(dialog).getAllByRole("button");
    const footerClose = buttons.find((b) => b.textContent === "Close")!;
    fireEvent.click(footerClose);
    await waitFor(() => {
      expect(screen.queryByTestId("rolling-restart-plan")).not.toBeInTheDocument();
    });
  });
});
