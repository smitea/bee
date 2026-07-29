import { describe, it, expect, vi, beforeEach } from "vitest";
import { render, screen, fireEvent } from "@testing-library/react";

const auditList = vi.fn();
const auditLatest = vi.fn();

vi.mock("../../ipc/audit", () => ({
  auditList,
  auditLatest,
  auditQuery: vi.fn(),
  auditRecord: vi.fn(),
}));

beforeEach(() => {
  vi.resetModules();
  auditList.mockReset();
  auditLatest.mockReset();
  auditList.mockResolvedValue([]);
  auditLatest.mockResolvedValue(null);
});

describe("<ActivityBar>", () => {
  it("renders the empty state when no events exist", async () => {
    const { ActivityBar } = await import("../../components/ActivityBar");
    render(<ActivityBar />);
    expect(screen.getByText("No activity yet")).toBeInTheDocument();
  });

  it("shows the latest event summary when present", async () => {
    auditList.mockResolvedValueOnce([
      {
        id: 1, timestamp: 1700000000, actor: "bee-client", action: "startup",
        result: "Success", summary: "Bee Client started",
        resource_kind: null, resource_id: null, application_id: null,
        correlation_id: null, operation_id: null,
        nav_kind: null, nav_resource_id: null,
      },
    ]);
    const { ActivityBar } = await import("../../components/ActivityBar");
    render(<ActivityBar />);
    expect(await screen.findByText("Bee Client started")).toBeInTheDocument();
  });

  it("clicking the bar opens the dialog", async () => {
    auditList.mockResolvedValueOnce([]);
    const { ActivityBar } = await import("../../components/ActivityBar");
    render(<ActivityBar />);
    fireEvent.click(screen.getByRole("button", { name: /open activity/i }));
    expect(screen.getByRole("dialog", { name: /activity/i })).toBeInTheDocument();
  });
});