import { beforeEach, describe, expect, it, vi } from "vitest";
import { fireEvent, render, screen, waitFor } from "@testing-library/react";
import { QueryClient, QueryClientProvider } from "@tanstack/react-query";

const mocks = vi.hoisted(() => ({
  tabsList: vi.fn(),
  tabOpen: vi.fn(),
  tabClose: vi.fn(),
  tabCloseOthers: vi.fn(),
  tabPin: vi.fn(),
  tabSetActive: vi.fn(),
  workspaceState: vi.fn(),
  applicationsList: vi.fn(),
}));

vi.mock("../../ipc/tabs", () => ({
  tabsList: mocks.tabsList,
  tabOpen: mocks.tabOpen,
  tabClose: mocks.tabClose,
  tabCloseOthers: mocks.tabCloseOthers,
  tabPin: mocks.tabPin,
  tabSetActive: mocks.tabSetActive,
  workspaceState: mocks.workspaceState,
}));

vi.mock("../../ipc/applications", () => ({
  applicationsList: mocks.applicationsList,
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
  connState: vi.fn(),
  ping: vi.fn(),
  getDefaultAddr: vi.fn(),
}));

vi.mock("../../ipc/audit", () => ({
  auditList: vi.fn().mockResolvedValue([]),
  auditStream: vi.fn(),
}));

import { AppShell } from "../../components/AppShell";
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
  vi.resetModules();
  mocks.tabsList.mockReset();
  mocks.tabOpen.mockReset();
  mocks.tabClose.mockReset();
  mocks.tabCloseOthers.mockReset();
  mocks.tabPin.mockReset();
  mocks.tabSetActive.mockReset();
  mocks.workspaceState.mockReset();
  mocks.applicationsList.mockReset();

  useTabs.setState({ tabs: [], activeId: null, hydrated: false });
  useApplications.setState({ items: [], loaded: false });

  mocks.tabsList.mockResolvedValue([
    {
      id: 1,
      kind: "cluster",
      resourceId: null,
      title: "Cluster",
      pinned: false,
      position: 0,
    },
    {
      id: 2,
      kind: "application_pipelines",
      resourceId: "1",
      title: "Pipelines",
      pinned: false,
      position: 1,
    },
    {
      id: 3,
      kind: "datasource",
      resourceId: "1",
      title: "ds",
      pinned: false,
      position: 2,
    },
  ]);
  mocks.workspaceState.mockResolvedValue({ activeTabId: 1 });
  mocks.applicationsList.mockResolvedValue([]);
  useConnection.setState({
    addr: "127.0.0.1:9999",
    status: { kind: "Connected" },
    hydrated: true,
  });
});

describe("<AppShell> tab context menu", () => {
  it("right-click on a tab opens the context menu", async () => {
    withClient(<AppShell />);
    const tab = await screen.findByRole("tab", { name: /pipelines/i });
    fireEvent.contextMenu(tab);
    expect(await screen.findByRole("menuitem", { name: /^close$/i })).toBeInTheDocument();
    expect(screen.getByRole("menuitem", { name: /^close others$/i })).toBeInTheDocument();
    expect(screen.getByRole("menuitem", { name: /close to the right/i })).toBeInTheDocument();
    expect(screen.getByRole("menuitem", { name: /^pin$/i })).toBeInTheDocument();
  });

  it("clicking Close in the context menu invokes tabClose for that tab", async () => {
    mocks.tabClose.mockResolvedValueOnce(undefined);
    mocks.tabsList.mockResolvedValueOnce([
      { id: 1, kind: "cluster", resourceId: null, title: "Cluster", pinned: false, position: 0 },
      { id: 2, kind: "application_pipelines", resourceId: "1", title: "Pipelines", pinned: false, position: 1 },
    ]);
    withClient(<AppShell />);
    const tab = await screen.findByRole("tab", { name: /pipelines/i });
    fireEvent.contextMenu(tab);
    fireEvent.click(await screen.findByRole("menuitem", { name: /^close$/i }));
    await waitFor(() => expect(mocks.tabClose).toHaveBeenCalledWith(2));
  });

  it("clicking Close Others invokes tabCloseOthers with the tab id", async () => {
    mocks.tabCloseOthers.mockResolvedValueOnce(undefined);
    withClient(<AppShell />);
    const tab = await screen.findByRole("tab", { name: /pipelines/i });
    fireEvent.contextMenu(tab);
    fireEvent.click(await screen.findByRole("menuitem", { name: /^close others$/i }));
    await waitFor(() => expect(mocks.tabCloseOthers).toHaveBeenCalledWith(2));
  });

  it("clicking Pin invokes tabPin with the inverse of the current pinned flag", async () => {
    mocks.tabPin.mockResolvedValueOnce(undefined);
    withClient(<AppShell />);
    const tab = await screen.findByRole("tab", { name: /pipelines/i });
    fireEvent.contextMenu(tab);
    fireEvent.click(await screen.findByRole("menuitem", { name: /pin/i }));
    await waitFor(() => expect(mocks.tabPin).toHaveBeenCalledWith(2, true));
  });

  it("Close to the Right is disabled for the rightmost tab", async () => {
    withClient(<AppShell />);
    const tab = await screen.findByRole("tab", { name: /^ds$/i });
    fireEvent.contextMenu(tab);
    const closeRight = await screen.findByRole("menuitem", { name: /close to the right/i });
    expect(closeRight).toBeDisabled();
  });
});