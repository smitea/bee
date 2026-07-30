import { beforeEach, describe, expect, it, vi } from "vitest";

const mocks = vi.hoisted(() => ({
  tabsList: vi.fn(),
  tabOpen: vi.fn(),
  tabClose: vi.fn(),
  tabCloseOthers: vi.fn(),
  tabPin: vi.fn(),
  tabSetActive: vi.fn(),
  workspaceState: vi.fn(),
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

import { useTabs } from "../../state/tabsStore";

function reset() {
  mocks.tabsList.mockReset();
  mocks.tabOpen.mockReset();
  mocks.tabClose.mockReset();
  mocks.tabCloseOthers.mockReset();
  mocks.tabPin.mockReset();
  mocks.tabSetActive.mockReset();
  mocks.workspaceState.mockReset();
  mocks.tabsList.mockResolvedValue([]);
  mocks.workspaceState.mockResolvedValue({ activeTabId: null });
  useTabs.setState({ tabs: [], activeId: null, hydrated: false });
}

describe("tabsStore closeOthers / closeRight", () => {
  beforeEach(reset);

  it("closeOthers invokes tabCloseOthers IPC and sets the kept id active", async () => {
    mocks.tabCloseOthers.mockResolvedValueOnce(undefined);
    mocks.tabsList.mockResolvedValueOnce([
      { id: 7, kind: "cluster", resourceId: null, title: "Cluster", pinned: false, position: 0 },
    ]);
    useTabs.setState({
      tabs: [
        { id: 7, kind: "cluster", resource_id: null, title: "Cluster", pinned: false, position: 0 },
        { id: 8, kind: "application_pipelines", resource_id: "1", title: "Pipelines", pinned: false, position: 1 },
        { id: 9, kind: "datasource", resource_id: "1", title: "ds", pinned: false, position: 2 },
      ],
      activeId: 9,
      hydrated: true,
    });

    await useTabs.getState().closeOthers(7);

    expect(mocks.tabCloseOthers).toHaveBeenCalledWith(7);
    expect(mocks.tabSetActive).toHaveBeenCalledWith(7);
    expect(useTabs.getState().activeId).toBe(7);
    expect(useTabs.getState().tabs.map((t) => t.id)).toEqual([7]);
  });

  it("closeRight closes only tabs whose position is greater than the given id", async () => {
    mocks.tabsList.mockResolvedValueOnce([
      { id: 1, kind: "cluster", resourceId: null, title: "Cluster", pinned: false, position: 0 },
      { id: 2, kind: "application", resourceId: "1", title: "App", pinned: false, position: 1 },
      { id: 3, kind: "datasource", resourceId: "1", title: "ds", pinned: false, position: 2 },
    ]);
    mocks.tabsList.mockResolvedValueOnce([
      { id: 1, kind: "cluster", resourceId: null, title: "Cluster", pinned: false, position: 0 },
      { id: 2, kind: "application", resourceId: "1", title: "App", pinned: false, position: 1 },
    ]);
    useTabs.setState({
      tabs: [
        { id: 1, kind: "cluster", resource_id: null, title: "Cluster", pinned: false, position: 0 },
        { id: 2, kind: "application", resource_id: "1", title: "App", pinned: false, position: 1 },
        { id: 3, kind: "datasource", resource_id: "1", title: "ds", pinned: false, position: 2 },
      ],
      activeId: 2,
      hydrated: true,
    });

    await useTabs.getState().closeRight(1);

    expect(mocks.tabClose).toHaveBeenCalledTimes(2);
    expect(mocks.tabClose.mock.calls.map((c) => c[0])).toEqual([2, 3]);
  });
});