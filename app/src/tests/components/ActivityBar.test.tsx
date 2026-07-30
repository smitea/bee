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

const tabOpen = vi.fn();
const tabsList = vi.fn();
const tabSetActive = vi.fn();
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
  auditLatest.mockReset();
  tabOpen.mockReset();
  tabsList.mockReset();
  tabSetActive.mockReset();
  auditList.mockResolvedValue([]);
  auditLatest.mockResolvedValue(null);
  tabsList.mockResolvedValue([]);
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

  it("clicking the bar opens the activity page (tab) and not the dialog", async () => {
    auditList.mockResolvedValueOnce([]);
    tabOpen.mockResolvedValueOnce(101);
    tabsList.mockResolvedValueOnce([
      {
        id: 101, kind: "activity", resource_id: null, title: "Recent Activity",
        pinned: false, position: 0,
      },
    ]);
    const { ActivityBar } = await import("../../components/ActivityBar");
    render(<ActivityBar />);
    fireEvent.click(screen.getByRole("button", { name: /click for full activity/i }));
    expect(tabOpen).toHaveBeenCalledWith("activity", null, "Recent Activity");
    expect(screen.queryByRole("dialog", { name: /^activity$/i })).toBeNull();
  });

  it("clicking the bar uses the navigate callback when provided instead of opening a tab", async () => {
    auditList.mockResolvedValueOnce([]);
    const navigate = vi.fn();
    const { ActivityBar } = await import("../../components/ActivityBar");
    render(<ActivityBar navigate={navigate} />);
    fireEvent.click(screen.getByRole("button", { name: /click for full activity/i }));
    expect(navigate).toHaveBeenCalledWith("activity", null);
    expect(tabOpen).not.toHaveBeenCalled();
  });
});
