import { describe, it, expect, vi, beforeEach } from "vitest";
import { QueryClient, QueryClientProvider } from "@tanstack/react-query";
import { render, screen, fireEvent } from "@testing-library/react";

const mocks = vi.hoisted(() => ({
  pipelineCreate: vi.fn(),
  openFn: vi.fn(),
  closeFn: vi.fn(),
  applicationsList: vi.fn(),
}));

vi.mock("../../ipc/pipelines", () => ({
  pipelineCreate: mocks.pipelineCreate,
  pipelineList: vi.fn(),
  pipelinesList: vi.fn(),
  pipelineGet: vi.fn(),
  pipelineDelete: vi.fn(),
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
  mocks.openFn.mockReset();
  mocks.closeFn.mockReset();
  mocks.applicationsList.mockReset();
  mocks.applicationsList.mockReturnValue([]);
  mocks.pipelineCreate.mockResolvedValue({
    id: 1,
    name: "alpha",
    dag_json: "{}",
    updated_at: 0,
  });
  mocks.openFn.mockResolvedValue(undefined);
  mocks.closeFn.mockResolvedValue(undefined);
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
  it("renders name input and DAG textarea", async () => {
    const { PipelineEditor } = await import("../../pages/PipelineEditor");
    withClient(<PipelineEditor />);
    expect(screen.getByPlaceholderText(/pipeline name/i)).toBeInTheDocument();
    expect(screen.getByLabelText(/dag_json/i)).toBeInTheDocument();
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
    fireEvent.change(screen.getByPlaceholderText(/pipeline name/i), {
      target: { value: "alpha" },
    });
    fireEvent.click(screen.getByRole("button", { name: /save pipeline/i }));
    await new Promise((r) => setTimeout(r, 50));
    expect(mocks.pipelineCreate).toHaveBeenCalled();
    const [passedName, passedDag] = mocks.pipelineCreate.mock.calls[0];
    expect(passedName).toBe("alpha");
    expect(typeof passedDag).toBe("string");
    JSON.parse(passedDag as string);
    expect(mocks.openFn).toHaveBeenCalled();
    const arg = mocks.openFn.mock.calls[0][0];
    expect(arg.kind).toBe("pipeline");
    expect(arg.resourceId).toBe("7");
  });

  it("shows an error when DAG JSON is invalid", async () => {
    const { PipelineEditor } = await import("../../pages/PipelineEditor");
    withClient(<PipelineEditor />);
    fireEvent.change(screen.getByPlaceholderText(/pipeline name/i), {
      target: { value: "alpha" },
    });
    fireEvent.change(screen.getByLabelText(/dag_json/i), {
      target: { value: "not json" },
    });
    fireEvent.click(screen.getByRole("button", { name: /save pipeline/i }));
    expect(await screen.findByRole("alert")).toBeInTheDocument();
    expect(mocks.pipelineCreate).not.toHaveBeenCalled();
  });

  it("cancel closes the editor tab", async () => {
    const { PipelineEditor } = await import("../../pages/PipelineEditor");
    withClient(<PipelineEditor />);
    fireEvent.click(screen.getByRole("button", { name: /cancel/i }));
    expect(mocks.closeFn).toHaveBeenCalled();
  });

  it("renders the PipelineGraph preview", async () => {
    const { PipelineEditor } = await import("../../pages/PipelineEditor");
    withClient(<PipelineEditor />);
    expect(screen.getByLabelText(/pipeline graph/i)).toBeInTheDocument();
  });
});
