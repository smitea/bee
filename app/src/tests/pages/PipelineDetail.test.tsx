import { describe, it, expect, vi, beforeEach } from "vitest";
import { QueryClient, QueryClientProvider } from "@tanstack/react-query";
import { render, screen, fireEvent, waitFor } from "@testing-library/react";

const mocks = vi.hoisted(() => {
  const openFn = vi.fn();
  const closeFn = vi.fn();
  return {
    pipelineGet: vi.fn(),
    pipelineList: vi.fn(),
    datasourceList: vi.fn(),
    jobInspect: vi.fn(),
    openFn,
    closeFn,
    tabOpen: vi.fn(),
  };
});

vi.mock("../../ipc/pipelines", () => ({
  pipelineGet: mocks.pipelineGet,
  pipelineList: mocks.pipelineList,
  pipelineCreate: vi.fn(),
  pipelineDelete: vi.fn(),
  pipelinesList: vi.fn(),
}));

vi.mock("../../ipc/datasources", () => ({
  datasourceList: mocks.datasourceList,
  datasourceCreate: vi.fn(),
  datasourceDelete: vi.fn(),
}));

vi.mock("../../ipc/cluster", () => ({
  jobInspect: mocks.jobInspect,
  listJobs: vi.fn(),
  clusterStatus: vi.fn(),
}));

function makeTabsApi() {
  const tabs = [
    {
      id: 42,
      kind: "pipeline" as const,
      resource_id: "7",
      title: "Pipeline 7",
      pinned: false,
      position: 0,
    },
  ];
  return {
    open: mocks.openFn,
    close: mocks.closeFn,
    tabs,
    activeId: 42,
  };
}

vi.mock("../../state/tabsStore", () => ({
  useTabs: (selector?: (s: ReturnType<typeof makeTabsApi>) => unknown) => {
    const state = makeTabsApi();
    return selector ? selector(state) : state;
  },
}));

beforeEach(() => {
  vi.resetModules();
  mocks.pipelineGet.mockReset();
  mocks.pipelineList.mockReset();
  mocks.datasourceList.mockReset();
  mocks.jobInspect.mockReset();
  mocks.openFn.mockReset();
  mocks.closeFn.mockReset();
  mocks.tabOpen.mockReset();

  mocks.pipelineList.mockResolvedValue([]);
  mocks.datasourceList.mockResolvedValue([]);
  mocks.jobInspect.mockResolvedValue(null);
  mocks.openFn.mockResolvedValue(undefined);
  mocks.closeFn.mockResolvedValue(undefined);
  mocks.tabOpen.mockResolvedValue(undefined);
});

function withClient(node: React.ReactNode) {
  const client = new QueryClient({
    defaultOptions: { queries: { retry: false, gcTime: 0 } },
  });
  return render(<QueryClientProvider client={client}>{node}</QueryClientProvider>);
}

const sample = {
  id: 7,
  name: "btc_pipeline",
  dag_json: JSON.stringify({
    input: {
      datasource: "binance",
      method: "subscribe",
      args: { symbol: "BTC/USDT" },
      output: "ticks",
    },
    handlers: [
      { id: "h1", name: "compute_kline", params: { window: "5m" }, upstream: ["ticks"] },
    ],
    output: { adapter: "kafka", method: "publish", args: {}, upstream: "h1" },
    crossPipelineRefs: [],
  }),
  updated_at: 1700000000,
};

describe("<PipelineDetail>", () => {
  it("shows the pipeline name and id in the header", async () => {
    mocks.pipelineGet.mockResolvedValueOnce(sample);
    const { PipelineDetail } = await import("../../pages/PipelineDetail");
    withClient(<PipelineDetail pipelineId={7} />);
    expect(await screen.findByText("btc_pipeline")).toBeInTheDocument();
    expect(screen.getByText(/#7/)).toBeInTheDocument();
  });

  it("renders the graph in default (Definition) mode", async () => {
    mocks.pipelineGet.mockResolvedValueOnce(sample);
    const { PipelineDetail } = await import("../../pages/PipelineDetail");
    withClient(<PipelineDetail pipelineId={7} />);
    expect(await screen.findByText(/compute_kline/)).toBeInTheDocument();
    expect(screen.getByText(/binance/)).toBeInTheDocument();
  });

  it("clicking a handler opens a side drawer with handler params", async () => {
    mocks.pipelineGet.mockResolvedValueOnce(sample);
    const { PipelineDetail } = await import("../../pages/PipelineDetail");
    withClient(<PipelineDetail pipelineId={7} />);
    fireEvent.click(await screen.findByText(/compute_kline/));
    expect(await screen.findByRole("dialog", { name: /handler/i })).toBeInTheDocument();
    expect(screen.getByText(/window/)).toBeInTheDocument();
    expect(screen.getByText(/5m/)).toBeInTheDocument();
  });

  it("clicking the input node opens a drawer that fetches datasources", async () => {
    mocks.pipelineGet.mockResolvedValueOnce(sample);
    mocks.datasourceList.mockResolvedValueOnce([
      { name: "binance", plugin: "binance_subscribe", config: "{}", tenant: 0, created_at: 0, updated_at: 0 },
    ]);
    const { PipelineDetail } = await import("../../pages/PipelineDetail");
    withClient(<PipelineDetail pipelineId={7} />);
    fireEvent.click(await screen.findByLabelText(/input node/i));
    await waitFor(() => expect(mocks.datasourceList).toHaveBeenCalled());
    expect(await screen.findByRole("dialog", { name: /datasource/i })).toBeInTheDocument();
  });

  it("clicking the input node falls back to a not-configured placeholder when no datasource exists", async () => {
    mocks.pipelineGet.mockResolvedValueOnce(sample);
    mocks.datasourceList.mockResolvedValueOnce([]);
    const { PipelineDetail } = await import("../../pages/PipelineDetail");
    withClient(<PipelineDetail pipelineId={7} />);
    fireEvent.click(await screen.findByLabelText(/input node/i));
    expect(await screen.findByText(/not configured/i)).toBeInTheDocument();
  });

  it("toggling to Runtime calls jobInspect and shows job details", async () => {
    mocks.pipelineGet.mockResolvedValueOnce(sample);
    mocks.jobInspect.mockResolvedValueOnce({
      job_id: 7,
      dag_hash: "abc",
      lifecycle: "Running",
      owner_node: 1,
      dependencies: [],
      tasks: [
        {
          task_id: 1,
          job_id: 7,
          phase_id: 1,
          status: "Running",
          owner_node: 1,
          started_at_ms: 0,
        },
      ],
    });
    const { PipelineDetail } = await import("../../pages/PipelineDetail");
    withClient(<PipelineDetail pipelineId={7} />);
    fireEvent.click(await screen.findByRole("button", { name: /runtime/i }));
    await waitFor(() => expect(mocks.jobInspect).toHaveBeenCalledWith("127.0.0.1:9999", 7));
    expect(await screen.findByText("Running")).toBeInTheDocument();
    expect(screen.getByText(/task #1/)).toBeInTheDocument();
  });

  it("clicking a cross-pipeline ref opens a new tab for the target pipeline", async () => {
    mocks.pipelineGet.mockResolvedValueOnce({
      ...sample,
      dag_json: JSON.stringify({
        input: { datasource: "binance", method: "subscribe", args: {}, output: "ticks" },
        handlers: [
          {
            id: "h1",
            name: "compute",
            params: {},
            upstream: ["ticks"],
          },
        ],
        output: { adapter: "kafka", method: "publish", args: {}, upstream: "h1" },
        crossPipelineRefs: [
          { upstreamPipelineName: "other_pipeline", upstreamPhaseId: "src", downstreamPhaseId: "h1" },
        ],
      }),
    });
    mocks.pipelineList.mockResolvedValue([
      { id: 9, name: "other_pipeline", dag_json: "{}", updated_at: 0 },
    ]);
    const { PipelineDetail } = await import("../../pages/PipelineDetail");
    withClient(<PipelineDetail pipelineId={7} />);
    fireEvent.click(await screen.findByLabelText(/cross-pipeline edge/i));
    await waitFor(() => expect(mocks.openFn).toHaveBeenCalled());
    const call = mocks.openFn.mock.calls[0][0];
    expect(call.kind).toBe("pipeline");
    expect(call.resourceId).toBe("9");
    expect(call.title).toBe("other_pipeline");
  });

  it("shows a rich empty state when the pipeline is not found", async () => {
    mocks.pipelineGet.mockResolvedValueOnce(null);
    const { PipelineDetail } = await import("../../pages/PipelineDetail");
    withClient(<PipelineDetail pipelineId={7} />);
    const alert = await screen.findByTestId("pipeline-not-found");
    expect(alert).toBeInTheDocument();
    expect(alert).toHaveTextContent(/Pipeline #7 not found/);
    expect(alert).toHaveTextContent(/deleted.*database was reset/i);
    expect(
      screen.getByRole("button", { name: /back to pipelines list/i }),
    ).toBeInTheDocument();
    expect(
      screen.getByRole("button", { name: /create new pipeline/i }),
    ).toBeInTheDocument();
  });

  it("'Back to Pipelines list' closes the current tab and opens the Pipelines page", async () => {
    mocks.pipelineGet.mockResolvedValueOnce(null);
    const { PipelineDetail } = await import("../../pages/PipelineDetail");
    withClient(<PipelineDetail pipelineId={7} />);
    fireEvent.click(await screen.findByRole("button", { name: /back to pipelines list/i }));
    await waitFor(() => {
      expect(mocks.closeFn).toHaveBeenCalledWith(42);
      expect(mocks.openFn).toHaveBeenCalled();
    });
    const call = mocks.openFn.mock.calls[0][0];
    expect(call.kind).toBe("application_pipelines");
    expect(call.title).toBe("Pipelines");
  });

  it("'Create new pipeline' opens a pipeline_editor tab", async () => {
    mocks.pipelineGet.mockResolvedValueOnce(null);
    const { PipelineDetail } = await import("../../pages/PipelineDetail");
    withClient(<PipelineDetail pipelineId={7} />);
    fireEvent.click(await screen.findByRole("button", { name: /create new pipeline/i }));
    await waitFor(() => expect(mocks.openFn).toHaveBeenCalled());
    const call = mocks.openFn.mock.calls[0][0];
    expect(call.kind).toBe("pipeline_editor");
    expect(call.title).toBe("New Pipeline");
  });

  it("renders an error alert with a retry button when pipelineGet fails", async () => {
    mocks.pipelineGet.mockRejectedValueOnce(new Error("boom"));
    const { PipelineDetail } = await import("../../pages/PipelineDetail");
    withClient(<PipelineDetail pipelineId={7} />);
    const alert = await screen.findByRole("alert");
    expect(alert).toHaveTextContent(/Failed to load pipeline/);
    expect(alert).toHaveTextContent(/boom/);
    const retry = screen.getByRole("button", { name: /retry/i });
    expect(retry).toBeInTheDocument();
    mocks.pipelineGet.mockResolvedValueOnce(sample);
    fireEvent.click(retry);
    await waitFor(() => expect(mocks.pipelineGet).toHaveBeenCalledTimes(2));
    expect(await screen.findByText("btc_pipeline")).toBeInTheDocument();
  });

  it("auto-closes the tab when a previously seen pipeline disappears", async () => {
    mocks.pipelineGet.mockResolvedValueOnce(sample);
    const client = new QueryClient({
      defaultOptions: { queries: { retry: false, gcTime: 0 } },
    });
    const { PipelineDetail } = await import("../../pages/PipelineDetail");
    render(
      <QueryClientProvider client={client}>
        <PipelineDetail pipelineId={7} />
      </QueryClientProvider>,
    );
    expect(await screen.findByText("btc_pipeline")).toBeInTheDocument();
    mocks.pipelineGet.mockResolvedValueOnce(null);
    await client.invalidateQueries({ queryKey: ["pipeline", 7] });
    await waitFor(() => expect(mocks.pipelineGet.mock.calls.length).toBeGreaterThanOrEqual(2));
    await screen.findByTestId("pipeline-not-found");
    await waitFor(() => expect(mocks.closeFn).toHaveBeenCalledWith(42));
  });
});
