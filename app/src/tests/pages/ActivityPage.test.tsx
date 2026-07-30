import { beforeEach, describe, expect, it, vi } from "vitest";
import { fireEvent, render, screen, waitFor, within } from "@testing-library/react";
import { QueryClient, QueryClientProvider } from "@tanstack/react-query";

const auditList = vi.fn();
const tabOpen = vi.fn();
const tabsList = vi.fn();
const tabSetActive = vi.fn();

vi.mock("../../ipc/audit", () => ({
  auditList,
  auditLatest: vi.fn(),
  auditQuery: vi.fn(),
  auditRecord: vi.fn(),
}));

vi.mock("../../ipc/tabs", () => ({
  tabOpen,
  tabsList,
  tabClose: vi.fn(),
  tabSetActive,
  tabPin: vi.fn(),
  workspaceState: vi.fn(),
}));

beforeEach(() => {
  vi.resetModules();
  auditList.mockReset();
  tabOpen.mockReset();
  tabsList.mockReset();
  tabSetActive.mockReset();
  auditList.mockResolvedValue([]);
  tabsList.mockResolvedValue([]);
});

function withClient(node: React.ReactNode) {
  const client = new QueryClient({
    defaultOptions: { queries: { retry: false, gcTime: 0 } },
  });
  return render(<QueryClientProvider client={client}>{node}</QueryClientProvider>);
}

function baseEvent(overrides: Partial<{
  id: number;
  timestamp: number;
  actor: string;
  action: string;
  result: string;
  summary: string;
  resource_kind: string | null;
  resource_id: string | null;
  application_id: number | null;
  correlation_id: string | null;
  operation_id: string | null;
  nav_kind: string | null;
  nav_resource_id: string | null;
}> = {}) {
  return {
    id: 1,
    timestamp: 1700000000,
    actor: "tester",
    action: "pipeline.deploy",
    result: "Success",
    summary: "Deployed pipeline p1",
    resource_kind: "pipeline",
    resource_id: "p1",
    application_id: null,
    correlation_id: null,
    operation_id: null,
    nav_kind: null,
    nav_resource_id: null,
    ...overrides,
  };
}

describe("<ActivityPage>", () => {
  it("renders audit events list from IPC", async () => {
    auditList.mockResolvedValueOnce([
      baseEvent({ id: 1, resource_id: "p1" }),
      baseEvent({
        id: 2,
        action: "pipeline.deploy",
        timestamp: 1700000010,
        resource_id: "p2",
      }),
      baseEvent({
        id: 3,
        action: "datasource.create",
        timestamp: 1700000020,
        resource_kind: "datasource",
        resource_id: "ds-1",
      }),
    ]);
    const { ActivityPage } = await import("../../pages/ActivityPage");
    withClient(<ActivityPage />);
    expect(await screen.findByText("datasource.create")).toBeInTheDocument();
    expect(screen.getAllByText("pipeline.deploy")).toHaveLength(2);
    expect(screen.getAllByTestId("activity-row")).toHaveLength(3);
    expect(screen.getByTestId("activity-page")).toBeInTheDocument();
    expect(auditList).toHaveBeenCalled();
  });

  it("filter by action category narrows the list", async () => {
    auditList.mockResolvedValueOnce([
      baseEvent({ id: 1, resource_id: "p1", action: "pipeline.deploy" }),
      baseEvent({
        id: 2,
        action: "datasource.create",
        resource_kind: "datasource",
        resource_id: "ds-1",
      }),
    ]);
    const { ActivityPage } = await import("../../pages/ActivityPage");
    withClient(<ActivityPage />);
    expect(await screen.findByText("datasource.create")).toBeInTheDocument();
    expect(screen.getAllByTestId("activity-row")).toHaveLength(2);
    fireEvent.change(screen.getByLabelText("Filter by action category"), {
      target: { value: "pipeline" },
    });
    expect(screen.getAllByTestId("activity-row")).toHaveLength(1);
    expect(screen.getByText("pipeline.deploy")).toBeInTheDocument();
    expect(screen.queryByText("datasource.create")).not.toBeInTheDocument();
  });

  it("clicking a row opens the detail dialog", async () => {
    auditList.mockResolvedValueOnce([
      baseEvent({
        id: 42,
        summary: "Inspectable event",
        action: "pipeline.deploy",
        actor: "auditor",
        resource_kind: "pipeline",
        resource_id: "p-42",
        correlation_id: "corr-42",
        operation_id: "op-42",
        application_id: 7,
      }),
    ]);
    const { ActivityPage } = await import("../../pages/ActivityPage");
    withClient(<ActivityPage />);
    expect(await screen.findByTestId("activity-row")).toBeInTheDocument();
    fireEvent.click(screen.getByTestId("activity-row"));
    const dialog = await screen.findByTestId("audit-event-detail-dialog");
    const dlg = within(dialog);
    expect(dlg.getByText("Inspectable event")).toBeInTheDocument();
    expect(dlg.getByText("pipeline.deploy")).toBeInTheDocument();
    expect(dlg.getByText("auditor")).toBeInTheDocument();
    expect(dlg.getByText("corr-42")).toBeInTheDocument();
    expect(dlg.getByText("op-42")).toBeInTheDocument();
  });

  it("shows an empty state when no events exist", async () => {
    auditList.mockResolvedValueOnce([]);
    const { ActivityPage } = await import("../../pages/ActivityPage");
    withClient(<ActivityPage />);
    expect(await screen.findByTestId("activity-empty")).toBeInTheDocument();
    const empty = screen.getByTestId("activity-empty");
    expect(empty.textContent).toMatch(/no activity yet/i);
  });

  it("registers useQuery with refetchInterval of 5000ms", async () => {
    auditList.mockResolvedValueOnce([]);
    const client = new QueryClient({
      defaultOptions: { queries: { retry: false, gcTime: 0 } },
    });
    const { ActivityPage } = await import("../../pages/ActivityPage");
    render(
      <QueryClientProvider client={client}>
        <ActivityPage />
      </QueryClientProvider>,
    );
    await waitFor(() => {
      const queries = client.getQueryCache().getAll();
      const audit = queries.find(
        (q) => Array.isArray(q.queryKey) && q.queryKey[0] === "audit-events",
      );
      expect(audit).toBeDefined();
    });
    const queries = client.getQueryCache().getAll();
    const audit = queries.find(
      (q) => Array.isArray(q.queryKey) && q.queryKey[0] === "audit-events",
    );
    expect(audit).toBeDefined();
    const refetchInterval = (audit!.options as { refetchInterval?: number })
      .refetchInterval;
    expect(refetchInterval).toBe(5000);
  });
});
