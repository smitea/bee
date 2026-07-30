import { describe, it, expect, vi, beforeEach } from "vitest";
import { render, screen, fireEvent } from "@testing-library/react";

const auditList = vi.fn();

vi.mock("../../ipc/audit", () => ({
  auditList,
  auditLatest: vi.fn(),
  auditQuery: vi.fn(),
  auditRecord: vi.fn(),
}));

const tabOpen = vi.fn();
const tabsList = vi.fn();
vi.mock("../../ipc/tabs", () => ({
  tabOpen,
  tabsList,
  tabClose: vi.fn(),
  tabSetActive: vi.fn(),
  tabPin: vi.fn(),
  workspaceState: vi.fn(),
}));

beforeEach(() => {
  vi.resetModules();
  auditList.mockReset();
  tabOpen.mockReset();
  tabsList.mockReset();
  tabsList.mockResolvedValue([]);
});

describe("<ActivityDialog>", () => {
  it("filters to errors only", async () => {
    auditList.mockResolvedValueOnce([
      {
        id: 1, timestamp: 1, actor: "x", action: "ok", result: "Success",
        summary: "good event", resource_kind: null, resource_id: null,
        application_id: null, correlation_id: null, operation_id: null,
        nav_kind: null, nav_resource_id: null,
      },
      {
        id: 2, timestamp: 2, actor: "x", action: "bad", result: "Failure",
        summary: "bad event", resource_kind: null, resource_id: null,
        application_id: null, correlation_id: null, operation_id: null,
        nav_kind: null, nav_resource_id: null,
      },
    ]);
    const { useAudit } = await import("../../state/auditStore");
    await useAudit.getState().refresh();
    const { ActivityDialog } = await import("../../components/ActivityDialog");
    render(<ActivityDialog onClose={() => {}} />);
    expect(screen.getByText("good event")).toBeInTheDocument();
    expect(screen.getByText("bad event")).toBeInTheDocument();
    fireEvent.change(screen.getByLabelText("Filter by result"), { target: { value: "errors" } });
    expect(screen.queryByText("good event")).not.toBeInTheDocument();
    expect(screen.getByText("bad event")).toBeInTheDocument();
  });

  it("Go-to button opens a navigation tab", async () => {
    auditList.mockResolvedValueOnce([
      {
        id: 1, timestamp: 1, actor: "x", action: "open", result: "Success",
        summary: "navigable", resource_kind: null, resource_id: null,
        application_id: null, correlation_id: null, operation_id: null,
        nav_kind: "application", nav_resource_id: "42",
      },
    ]);
    tabOpen.mockResolvedValueOnce(7);
    const { useAudit } = await import("../../state/auditStore");
    await useAudit.getState().refresh();
    const { ActivityDialog } = await import("../../components/ActivityDialog");
    render(<ActivityDialog onClose={() => {}} />);
    fireEvent.click(screen.getByRole("button", { name: /go to application/i }));
    expect(tabOpen).toHaveBeenCalledWith("application", "42", "navigable");
  });

  it("renders event summary and full details (header + key/value table)", async () => {
    auditList.mockResolvedValueOnce([
      {
        id: 1, timestamp: 1700000000, actor: "tester", action: "pipeline.deploy",
        result: "Success", summary: "Deployed pipeline p1",
        resource_kind: "pipeline", resource_id: "p1",
        application_id: 7, correlation_id: "corr-1", operation_id: "op-1",
        nav_kind: null, nav_resource_id: null,
      },
    ]);
    const { useAudit } = await import("../../state/auditStore");
    await useAudit.getState().refresh();
    const { ActivityDialog } = await import("../../components/ActivityDialog");
    render(<ActivityDialog onClose={() => {}} />);
    expect(screen.getByText("Deployed pipeline p1")).toBeInTheDocument();
    expect(screen.getByText("pipeline.deploy")).toBeInTheDocument();
    expect(screen.getByText("tester")).toBeInTheDocument();
    expect(screen.getByText("pipeline", { selector: "span" })).toBeInTheDocument();
    expect(screen.getByText("p1")).toBeInTheDocument();
    expect(screen.getByText("corr-1")).toBeInTheDocument();
    expect(screen.getByText("op-1")).toBeInTheDocument();
    expect(screen.getByText("application_id")).toBeInTheDocument();
  });

  it("filters by action category", async () => {
    auditList.mockResolvedValueOnce([
      {
        id: 1, timestamp: 1, actor: "x", action: "pipeline.deploy", result: "Success",
        summary: "Pipeline event", resource_kind: null, resource_id: null,
        application_id: null, correlation_id: null, operation_id: null,
        nav_kind: null, nav_resource_id: null,
      },
      {
        id: 2, timestamp: 2, actor: "x", action: "datasource.create", result: "Success",
        summary: "Datasource event", resource_kind: null, resource_id: null,
        application_id: null, correlation_id: null, operation_id: null,
        nav_kind: null, nav_resource_id: null,
      },
    ]);
    const { useAudit } = await import("../../state/auditStore");
    await useAudit.getState().refresh();
    const { ActivityDialog } = await import("../../components/ActivityDialog");
    render(<ActivityDialog onClose={() => {}} />);
    expect(screen.getByText("Pipeline event")).toBeInTheDocument();
    expect(screen.getByText("Datasource event")).toBeInTheDocument();
    fireEvent.change(screen.getByLabelText("Filter by action category"), {
      target: { value: "pipeline" },
    });
    expect(screen.getByText("Pipeline event")).toBeInTheDocument();
    expect(screen.queryByText("Datasource event")).not.toBeInTheDocument();
  });

  it("Go-to button calls navigate callback with the right args", async () => {
    auditList.mockResolvedValueOnce([
      {
        id: 1, timestamp: 1, actor: "x", action: "pipeline.deploy", result: "Success",
        summary: "Deployed", resource_kind: "pipeline", resource_id: "JOB-42",
        application_id: null, correlation_id: null, operation_id: null,
        nav_kind: null, nav_resource_id: null,
      },
    ]);
    const navigate = vi.fn();
    const { useAudit } = await import("../../state/auditStore");
    await useAudit.getState().refresh();
    const { ActivityDialog } = await import("../../components/ActivityDialog");
    render(<ActivityDialog onClose={() => {}} navigate={navigate} />);
    fireEvent.click(screen.getByRole("button", { name: /open pipeline JOB-42/i }));
    expect(navigate).toHaveBeenCalledWith("application_pipelines", "JOB-42");
  });

  it("hides the go-to button when event has no nav_action", async () => {
    auditList.mockResolvedValueOnce([
      {
        id: 1, timestamp: 1, actor: "x", action: "cluster.connection.activate",
        result: "Success", summary: "Activated",
        resource_kind: null, resource_id: null,
        application_id: null, correlation_id: null, operation_id: null,
        nav_kind: null, nav_resource_id: null,
      },
    ]);
    const { useAudit } = await import("../../state/auditStore");
    await useAudit.getState().refresh();
    const { ActivityDialog } = await import("../../components/ActivityDialog");
    render(<ActivityDialog onClose={() => {}} />);
    expect(screen.queryByRole("button", { name: /open /i })).toBeNull();
    expect(screen.queryByRole("button", { name: /go to /i })).toBeNull();
  });
});
