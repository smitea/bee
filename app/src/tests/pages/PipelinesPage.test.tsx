import { describe, it, expect, vi, beforeEach } from "vitest";
import { QueryClient, QueryClientProvider } from "@tanstack/react-query";
import { render, screen, fireEvent } from "@testing-library/react";

const mocks = vi.hoisted(() => ({
  pipelineList: vi.fn(),
  pipelineCreate: vi.fn(),
  pipelineDelete: vi.fn(),
  listJobs: vi.fn(),
  openFn: vi.fn(),
}));

vi.mock("../../ipc/pipelines", () => ({
  pipelineList: mocks.pipelineList,
  pipelineCreate: mocks.pipelineCreate,
  pipelineDelete: mocks.pipelineDelete,
  pipelineGet: vi.fn(),
  pipelinesList: vi.fn(),
}));

vi.mock("../../ipc/cluster", () => ({
  listJobs: mocks.listJobs,
  clusterStatus: vi.fn(),
  jobInspect: vi.fn(),
}));

function makeTabsApi() {
  return { open: mocks.openFn };
}

vi.mock("../../state/tabsStore", () => ({
  useTabs: (selector?: (s: ReturnType<typeof makeTabsApi>) => unknown) => {
    const state = makeTabsApi();
    return selector ? selector(state) : state;
  },
}));

beforeEach(() => {
  vi.resetModules();
  mocks.pipelineList.mockReset();
  mocks.pipelineCreate.mockReset();
  mocks.pipelineDelete.mockReset();
  mocks.listJobs.mockReset();
  mocks.openFn.mockReset();

  mocks.pipelineList.mockResolvedValue([]);
  mocks.listJobs.mockResolvedValue([]);
  mocks.openFn.mockResolvedValue(undefined);
});

function withClient(node: React.ReactNode) {
  const client = new QueryClient({
    defaultOptions: { queries: { retry: false, gcTime: 0 } },
  });
  return render(<QueryClientProvider client={client}>{node}</QueryClientProvider>);
}

describe("<PipelinesPage>", () => {
  it("renders all four section headings even when empty", async () => {
    const { PipelinesPage } = await import("../../pages/PipelinesPage");
    withClient(<PipelinesPage />);
    expect(await screen.findByText(/^Queued/)).toBeInTheDocument();
    expect(screen.getByText(/^Running/)).toBeInTheDocument();
    expect(screen.getByText(/^Historical/)).toBeInTheDocument();
    expect(screen.getByText(/^Failed/)).toBeInTheDocument();
  });

  it("partitions jobs into the correct sections", async () => {
    mocks.listJobs.mockResolvedValueOnce([
      { job_id: 1, dag_hash: "x", lifecycle: "WaitingForUpstream", mode: "x", task_count: 0, owner_node: 1 },
      { job_id: 2, dag_hash: "y", lifecycle: "Running", mode: "x", task_count: 1, owner_node: 1 },
      { job_id: 3, dag_hash: "z", lifecycle: "Completed", mode: "x", task_count: 1, owner_node: 1 },
      { job_id: 4, dag_hash: "w", lifecycle: "Failed", mode: "x", task_count: 1, owner_node: 1 },
    ]);
    mocks.pipelineList.mockResolvedValueOnce([]);
    const { PipelinesPage } = await import("../../pages/PipelinesPage");
    withClient(<PipelinesPage />);
    expect(await screen.findByText(/#1/)).toBeInTheDocument();
    expect(screen.getByText(/#2/)).toBeInTheDocument();
    expect(screen.getByText(/#3/)).toBeInTheDocument();
    expect(screen.getByText(/#4/)).toBeInTheDocument();
  });

  it("create-new button opens a pipeline tab when name is provided", async () => {
    mocks.pipelineCreate.mockResolvedValueOnce({
      id: 11,
      name: "btc_kline",
      dag_json: "{}",
      updated_at: 0,
    });
    const { PipelinesPage } = await import("../../pages/PipelinesPage");
    withClient(<PipelinesPage />);
    const nameInput = await screen.findByPlaceholderText("btc_kline");
    fireEvent.change(nameInput, { target: { value: "btc_kline" } });
    fireEvent.click(screen.getByRole("button", { name: /create.*pipeline/i }));
    await new Promise((r) => setTimeout(r, 50));
    expect(mocks.openFn).toHaveBeenCalled();
    const arg = mocks.openFn.mock.calls[0][0];
    expect(arg.kind).toBe("pipeline");
    expect(arg.resourceId).toBe("11");
    expect(arg.title).toBe("btc_kline");
  });

  it("opens a tab when clicking an existing pipeline row", async () => {
    mocks.pipelineList.mockResolvedValueOnce([
      { id: 42, name: "alpha", dag_json: "{}", updated_at: 0 },
    ]);
    const { PipelinesPage } = await import("../../pages/PipelinesPage");
    withClient(<PipelinesPage />);
    fireEvent.click(await screen.findByText("alpha"));
    expect(mocks.openFn).toHaveBeenCalled();
    const arg = mocks.openFn.mock.calls[0][0];
    expect(arg.kind).toBe("pipeline");
    expect(arg.resourceId).toBe("42");
  });
});
