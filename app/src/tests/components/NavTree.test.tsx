import { describe, it, expect, vi, beforeEach } from "vitest";
import { render, screen, fireEvent } from "@testing-library/react";
import { QueryClient, QueryClientProvider } from "@tanstack/react-query";

const applicationsList = vi.fn();
const applicationCreate = vi.fn();
const applicationSetEnabled = vi.fn();
const applicationDelete = vi.fn();

const tabsList = vi.fn();
const tabOpen = vi.fn();
const tabClose = vi.fn();
const tabSetActive = vi.fn();
const tabPin = vi.fn();
const workspaceState = vi.fn();

const tenantGet = vi.fn();
const tenantSet = vi.fn();

const clusterProfileList = vi.fn();
const clusterProfileSave = vi.fn();
const clusterProfileRemove = vi.fn();
const clusterProfileActivate = vi.fn();
const clusterProfileMigrateLegacy = vi.fn();

vi.mock("../../ipc/applications", () => ({
  applicationsList,
  applicationCreate,
  applicationSetEnabled,
  applicationDelete,
}));
vi.mock("../../ipc/tabs", () => ({
  tabsList,
  tabOpen,
  tabClose,
  tabSetActive,
  tabPin,
  workspaceState,
}));
vi.mock("../../ipc/tenant", () => ({
  tenantGet,
  tenantSet,
}));
vi.mock("../../ipc/clusters", () => ({
  clusterProfileList,
  clusterProfileSave,
  clusterProfileRemove,
  clusterProfileActivate,
  clusterProfileMigrateLegacy,
}));

beforeEach(() => {
  vi.resetModules();
  applicationsList.mockReset();
  applicationCreate.mockReset();
  applicationSetEnabled.mockReset();
  applicationDelete.mockReset();
  tabsList.mockReset();
  tabOpen.mockReset();
  tabClose.mockReset();
  tabSetActive.mockReset();
  tabPin.mockReset();
  workspaceState.mockReset();

  tenantGet.mockReset();
  tenantSet.mockReset();
  clusterProfileList.mockReset();
  clusterProfileSave.mockReset();
  clusterProfileRemove.mockReset();
  clusterProfileActivate.mockReset();
  clusterProfileMigrateLegacy.mockReset();

  applicationsList.mockResolvedValue([]);
  tabsList.mockResolvedValue([]);
  workspaceState.mockResolvedValue({ activeTabId: null });
  tenantGet.mockResolvedValue(0);
  tenantSet.mockResolvedValue(0);
  clusterProfileList.mockResolvedValue([]);
  clusterProfileMigrateLegacy.mockResolvedValue({ inserted: 0, skipped: [] });

  if (typeof window !== "undefined" && typeof window.confirm === "function") {
    vi.spyOn(window, "confirm").mockReturnValue(true);
  }
});

function withClient(node: React.ReactNode) {
  const client = new QueryClient({
    defaultOptions: { queries: { retry: false, gcTime: 0 } },
  });
  return render(<QueryClientProvider client={client}>{node}</QueryClientProvider>);
}

describe("<NavTree>", () => {
  it("renders Cluster row + Application count of zero", async () => {
    const { NavTree } = await import("../../components/NavTree");
    withClient(<NavTree />);
    expect(screen.getByText("Cluster")).toBeInTheDocument();
    expect(screen.getByText(/Applications \(0\)/)).toBeInTheDocument();
    expect(screen.getByText(/No applications yet/)).toBeInTheDocument();
  });

  it("clicking Cluster opens a cluster tab", async () => {
    tabOpen.mockResolvedValueOnce(99);
    tabsList.mockResolvedValueOnce([{ id: 99, kind: "cluster", resource_id: null, title: "Cluster", pinned: false, position: 1 }]);
    const { NavTree } = await import("../../components/NavTree");
    withClient(<NavTree />);
    fireEvent.click(screen.getByText("Cluster"));
    expect(tabOpen).toHaveBeenCalledWith("cluster", null, "Cluster");
  });

  it("lists existing applications", async () => {
    applicationsList.mockResolvedValue([
      { id: 1, name: "alpha", enabled: true, display_order: 1, tenant: 0, created_at: 0 },
      { id: 2, name: "beta", enabled: false, display_order: 2, tenant: 5, created_at: 0 },
    ]);
    const { useApplications } = await import("../../state/applicationsStore");
    await useApplications.getState().refresh();
    const { NavTree } = await import("../../components/NavTree");
    withClient(<NavTree />);
    expect(await screen.findByText("alpha")).toBeInTheDocument();
    expect(screen.getByText("beta")).toBeInTheDocument();
    expect(screen.getByText(/Applications \(2\)/)).toBeInTheDocument();
  });

  it("does not render the legacy Close-other-tabs / Toggle-pinned sidebar buttons", async () => {
    const { NavTree } = await import("../../components/NavTree");
    withClient(<NavTree />);
    expect(screen.queryByText(/close other tabs/i)).toBeNull();
    expect(screen.queryByText(/toggle pinned/i)).toBeNull();
  });

  it("renders the Add-application control inside the Applications row (not the Bee header)", async () => {
    const { NavTree } = await import("../../components/NavTree");
    withClient(<NavTree />);
    const appsHeading = screen.getByText(/Applications \(0\)/);
    const add = screen.getByRole("button", { name: /add application/i });
    expect(appsHeading.parentElement?.contains(add)).toBe(true);
    expect(screen.getByText("Bee").closest("div")?.querySelector('[aria-label="Add application"]')).toBeNull();
  });
});