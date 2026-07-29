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
    fireEvent.change(screen.getByRole("combobox"), { target: { value: "errors" } });
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
});