import { beforeEach, describe, expect, it, vi } from "vitest";
import { render, screen, waitFor } from "@testing-library/react";
import { QueryClient, QueryClientProvider } from "@tanstack/react-query";

const mocks = vi.hoisted(() => ({
  pipelineCreate: vi.fn(),
  pipelineGet: vi.fn(),
  pipelineDelete: vi.fn(),
  openFn: vi.fn(),
  closeFn: vi.fn(),
  jobInspect: vi.fn(),
  pipelineDumpRecord: vi.fn(),
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
  useTabs: () => ({
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
  }),
}));

vi.mock("../../state/applicationsStore", () => ({
  useApplications: () => ({ items: [] }),
}));

beforeEach(() => {
  mocks.pipelineCreate.mockReset();
  mocks.pipelineGet.mockReset();
  mocks.pipelineDelete.mockReset();
  mocks.openFn.mockReset();
  mocks.closeFn.mockReset();
  mocks.jobInspect.mockReset();
  mocks.pipelineDumpRecord.mockReset();
  mocks.pipelineCreate.mockResolvedValue({ id: 1, name: "alpha", dag_json: "{}", updated_at: 0 });
  mocks.pipelineGet.mockResolvedValue(null);
  mocks.openFn.mockResolvedValue(undefined);
  mocks.closeFn.mockResolvedValue(undefined);
  mocks.jobInspect.mockResolvedValue(null);
  mocks.pipelineDumpRecord.mockResolvedValue({ pipeline_id: 1, dump_json: "{}", created_at: 0 });
});

function withClient(node: React.ReactNode) {
  const client = new QueryClient({
    defaultOptions: { queries: { retry: false, gcTime: 0 } },
  });
  return render(<QueryClientProvider client={client}>{node}</QueryClientProvider>);
}

describe("PipelineEditor code-split boundaries", () => {
  it("renders the name input and tab buttons without waiting for CodeMirror chunk", async () => {
    const { PipelineEditor } = await import("../../pages/PipelineEditor");
    withClient(<PipelineEditor />);

    expect(screen.getByTestId("pipeline-name")).toBeInTheDocument();
    expect(screen.getByTestId("tab-dag")).toBeInTheDocument();
    expect(screen.getByTestId("tab-sql")).toBeInTheDocument();
  });

  it("falls back to the loading skeleton when the CodeMirror chunk has not loaded", async () => {
    let resolveChunk: (mod: typeof import("../../pages/PipelineCodePane")) => void = () => {};
    const pendingChunk = new Promise<typeof import("../../pages/PipelineCodePane")>(
      (resolve) => {
        resolveChunk = resolve;
      },
    );
    vi.doMock("../../pages/PipelineCodePane", () => pendingChunk);

    try {
      const { PipelineEditor } = await import("../../pages/PipelineEditor");
      withClient(<PipelineEditor />);

      await waitFor(() =>
        expect(screen.queryByTestId("pipeline-editor-fallback")).toBeTruthy(),
      );
      resolveChunk({
        PipelineCodePane: () => <div data-testid="code-stub">stub</div>,
      });
    } finally {
      vi.doUnmock("../../pages/PipelineCodePane");
    }
  });

  it("falls back to the loading skeleton when the DAG designer chunk has not loaded", async () => {
    let resolveChunk: (mod: typeof import("../../pages/PipelineDagPane")) => void = () => {};
    const pendingChunk = new Promise<typeof import("../../pages/PipelineDagPane")>(
      (resolve) => {
        resolveChunk = resolve;
      },
    );
    vi.doMock("../../pages/PipelineDagPane", () => pendingChunk);

    try {
      const { PipelineEditor } = await import("../../pages/PipelineEditor");
      withClient(<PipelineEditor />);

      await waitFor(() =>
        expect(screen.queryByTestId("pipeline-editor-fallback")).toBeTruthy(),
      );
      resolveChunk({
        PipelineDagPane: () => <div data-testid="dag-stub">stub</div>,
      });
    } finally {
      vi.doUnmock("../../pages/PipelineDagPane");
    }
  });
});