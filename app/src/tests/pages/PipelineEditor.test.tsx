import { describe, it, expect, vi, beforeEach } from "vitest";
import { QueryClient, QueryClientProvider } from "@tanstack/react-query";
import { render, screen, fireEvent, waitFor } from "@testing-library/react";

const mocks = vi.hoisted(() => ({
  pipelineCreate: vi.fn(),
  pipelineGet: vi.fn(),
  pipelineDelete: vi.fn(),
  openFn: vi.fn(),
  closeFn: vi.fn(),
  jobInspect: vi.fn(),
  pipelineDumpRecord: vi.fn(),
  applicationsList: vi.fn(),
}));

vi.mock("../../ipc/pipelines", () => ({
  pipelineCreate: mocks.pipelineCreate,
  pipelineList: vi.fn(),
  pipelinesList: vi.fn(),
  pipelineGet: mocks.pipelineGet,
  pipelineDelete: mocks.pipelineDelete,
  pipelineLatestResult: vi.fn(),
}));

vi.mock("../../ipc/cluster", () => ({
  jobInspect: mocks.jobInspect,
  clusterStatus: vi.fn(),
  listJobs: vi.fn(),
}));

vi.mock("../../ipc/pipeline_dumps", () => ({
  pipelineDumpRecord: mocks.pipelineDumpRecord,
  pipelineDumpList: vi.fn(),
}));

vi.mock("../../state/tabsStore", () => ({
  useTabs: (selector?: (s: ReturnType<typeof makeTabsApi>) => unknown) => {
    const state = makeTabsApi();
    return selector ? selector(state) : state;
  },
}));

vi.mock("../../state/applicationsStore", () => ({
  useApplications: (selector?: (s: { items: { id: number; name: string }[] }) => unknown) => {
    const state = { items: mocks.applicationsList.mock.results[0]?.value ?? [] };
    return selector ? selector(state) : state;
  },
}));

function makeTabsApi() {
  return {
    open: mocks.openFn,
    close: mocks.closeFn,
    tabs: [
      {
        id: 1,
        kind: "pipeline_editor" as const,
        resource_id: null,
        title: "New Pipeline",
        pinned: false,
        position: 0,
      },
    ],
    activeId: 1,
  };
}

beforeEach(() => {
  vi.resetModules();
  mocks.pipelineCreate.mockReset();
  mocks.pipelineGet.mockReset();
  mocks.pipelineDelete.mockReset();
  mocks.openFn.mockReset();
  mocks.closeFn.mockReset();
  mocks.jobInspect.mockReset();
  mocks.pipelineDumpRecord.mockReset();
  mocks.applicationsList.mockReset();
  mocks.applicationsList.mockReturnValue([]);
  mocks.pipelineCreate.mockResolvedValue({
    id: 1,
    name: "alpha",
    dag_json: "{}",
    updated_at: 0,
  });
  mocks.pipelineGet.mockResolvedValue(null);
  mocks.openFn.mockResolvedValue(undefined);
  mocks.closeFn.mockResolvedValue(undefined);
  mocks.jobInspect.mockResolvedValue(null);
  mocks.pipelineDumpRecord.mockResolvedValue({
    pipeline_id: 1,
    dump_json: "{}",
    created_at: 0,
  });
});

function withClient(node: React.ReactNode) {
  const client = new QueryClient({
    defaultOptions: { queries: { retry: false, gcTime: 0 } },
  });
  return render(
    <QueryClientProvider client={client}>{node}</QueryClientProvider>,
  );
}

describe("<PipelineEditor>", () => {
  it("renders name input and DAG tab with CodeMirror + DAG designer", async () => {
    const { PipelineEditor } = await import("../../pages/PipelineEditor");
    withClient(<PipelineEditor />);
    expect(screen.getByTestId("pipeline-name")).toBeInTheDocument();
    expect(screen.getByTestId("dag-designer")).toBeInTheDocument();
  });

  it("switches between SQL and DAG tabs", async () => {
    const { PipelineEditor } = await import("../../pages/PipelineEditor");
    withClient(<PipelineEditor />);
    fireEvent.click(screen.getByTestId("tab-sql"));
    expect(screen.getByTestId("tab-sql")).toBeInTheDocument();
    fireEvent.click(screen.getByTestId("tab-dag"));
    expect(screen.getByTestId("tab-dag")).toBeInTheDocument();
  });

  it("save calls pipelineCreate and opens the new pipeline tab", async () => {
    mocks.pipelineCreate.mockResolvedValueOnce({
      id: 7,
      name: "alpha",
      dag_json: "{}",
      updated_at: 0,
    });
    const { PipelineEditor } = await import("../../pages/PipelineEditor");
    withClient(<PipelineEditor />);
    fireEvent.change(screen.getByTestId("pipeline-name"), {
      target: { value: "alpha" },
    });
    fireEvent.click(screen.getByTestId("pipeline-save"));
    await waitFor(() => expect(mocks.pipelineCreate).toHaveBeenCalled());
    const [passedName] = mocks.pipelineCreate.mock.calls[0];
    expect(passedName).toBe("alpha");
    expect(mocks.openFn).toHaveBeenCalled();
    const arg = mocks.openFn.mock.calls[0][0];
    expect(arg.kind).toBe("pipeline");
    expect(arg.resourceId).toBe("7");
  });

  it("renders save button enabled", async () => {
    const { PipelineEditor } = await import("../../pages/PipelineEditor");
    withClient(<PipelineEditor />);
    expect(screen.getByTestId("pipeline-save")).toBeInTheDocument();
  });

  it("cancel closes the editor tab", async () => {
    const { PipelineEditor } = await import("../../pages/PipelineEditor");
    withClient(<PipelineEditor />);
    fireEvent.click(screen.getByTestId("pipeline-cancel"));
    expect(mocks.closeFn).toHaveBeenCalled();
  });

  it("debug button runs a synthetic tick", async () => {
    mocks.jobInspect.mockResolvedValueOnce({
      job_id: 1,
      dag_hash: "x",
      lifecycle: "Running",
      owner_node: 1,
      dependencies: [],
      tasks: [],
    });
    const { PipelineEditor } = await import("../../pages/PipelineEditor");
    withClient(<PipelineEditor />);
    fireEvent.change(screen.getByTestId("pipeline-name"), {
      target: { value: "alpha" },
    });
    fireEvent.click(screen.getByTestId("pipeline-debug"));
    await waitFor(() => expect(mocks.jobInspect).toHaveBeenCalled());
  });

  it("dump button records a dump via pipeline_dump_record", async () => {
    const { PipelineEditor } = await import("../../pages/PipelineEditor");
    withClient(<PipelineEditor />);
    fireEvent.change(screen.getByTestId("pipeline-name"), {
      target: { value: "alpha" },
    });
    fireEvent.click(screen.getByTestId("pipeline-dump"));
    await waitFor(() => expect(mocks.pipelineDumpRecord).toHaveBeenCalled());
  });
});