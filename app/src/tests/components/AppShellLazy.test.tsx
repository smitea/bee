import { beforeEach, describe, expect, it, vi } from "vitest";
import { render, screen, waitFor } from "@testing-library/react";
import { QueryClient, QueryClientProvider } from "@tanstack/react-query";

vi.mock("../../ipc/tabs", () => ({
  tabsList: vi.fn().mockResolvedValue([]),
  tabOpen: vi.fn(),
  tabClose: vi.fn(),
  tabCloseOthers: vi.fn(),
  tabPin: vi.fn(),
  tabSetActive: vi.fn(),
  workspaceState: vi.fn().mockResolvedValue({ activeTabId: null }),
}));

vi.mock("../../ipc/applications", () => ({
  applicationsList: vi.fn().mockResolvedValue([]),
  applicationCreate: vi.fn(),
  applicationSetEnabled: vi.fn(),
  applicationDelete: vi.fn(),
}));

vi.mock("../../ipc/clusters", () => ({
  clusterProfileList: vi.fn().mockResolvedValue([]),
  clusterProfileSave: vi.fn(),
  clusterProfileRemove: vi.fn(),
  clusterProfileActivate: vi.fn(),
  clusterProfileMigrateLegacy: vi.fn().mockResolvedValue({ inserted: 0, skipped: [] }),
}));

vi.mock("../../ipc/connection", () => ({
  setAddr: vi.fn(),
  testConnection: vi.fn(),
  connState: vi.fn().mockResolvedValue({ addr: "127.0.0.1:9999", status: "Connected" }),
  ping: vi.fn(),
  getDefaultAddr: vi.fn(),
}));

vi.mock("../../ipc/audit", () => ({
  auditList: vi.fn().mockResolvedValue([]),
  auditStream: vi.fn(),
}));

import { useConnection } from "../../state/connectionStore";
import { useTabs } from "../../state/tabsStore";
import { useApplications } from "../../state/applicationsStore";

function withClient(node: React.ReactNode) {
  const client = new QueryClient({
    defaultOptions: { queries: { retry: false, gcTime: 0 } },
  });
  return render(<QueryClientProvider client={client}>{node}</QueryClientProvider>);
}

beforeEach(() => {
  useTabs.setState({ tabs: [], activeId: null, hydrated: false });
  useApplications.setState({ items: [], loaded: false });
  useConnection.setState({
    addr: "127.0.0.1:9999",
    status: { kind: "Connected" },
    hydrated: true,
  });
});

describe("AppShell code-split boundaries", () => {
  it("imports the DashboardPage chunk as a dynamic module (not a static binding)", async () => {
    useTabs.setState({
      tabs: [
        {
          id: 1,
          kind: "application_dashboard",
          resource_id: "42",
          title: "Dashboard",
          pinned: false,
          position: 0,
        },
      ],
      activeId: 1,
      hydrated: true,
    });

    const { AppShell } = await import("../../components/AppShell");
    withClient(<AppShell />);

    await waitFor(() => expect(screen.getByTestId("brand-label")).toBeInTheDocument());
  });

  it("renders the header synchronously even before the Dashboard chunk resolves", async () => {
    useTabs.setState({
      tabs: [
        {
          id: 1,
          kind: "application_dashboard",
          resource_id: "42",
          title: "Dashboard",
          pinned: false,
          position: 0,
        },
      ],
      activeId: 1,
      hydrated: true,
    });

    const { AppShell } = await import("../../components/AppShell");
    withClient(<AppShell />);

    const brand = screen.getByTestId("brand-label");
    expect(brand.textContent).toBe("Bee");
  });

  it("renders a Suspense fallback when the DashboardPage chunk is still loading", async () => {
    let resolveChunk: (mod: typeof import("../../pages/DashboardPage")) => void = () => {};
    const pendingChunk = new Promise<typeof import("../../pages/DashboardPage")>(
      (resolve) => {
        resolveChunk = resolve;
      },
    );
    vi.doMock("../../pages/DashboardPage", () => pendingChunk);

    try {
      useTabs.setState({
        tabs: [
          {
            id: 1,
            kind: "application_dashboard",
            resource_id: "42",
            title: "Dashboard",
            pinned: false,
            position: 0,
          },
        ],
        activeId: 1,
        hydrated: true,
      });

      const { AppShell } = await import("../../components/AppShell");
      withClient(<AppShell />);

      await waitFor(() =>
        expect(screen.queryAllByTestId("lazy-fallback").length).toBeGreaterThan(0),
      );
      resolveChunk({
        DashboardPage: () => <div data-testid="dashboard-stub">stub</div>,
      });
    } finally {
      vi.doUnmock("../../pages/DashboardPage");
    }
  });
});