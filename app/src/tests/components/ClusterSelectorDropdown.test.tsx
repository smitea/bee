import { beforeEach, describe, expect, it, vi } from "vitest";
import { fireEvent, render, screen, waitFor } from "@testing-library/react";
import { QueryClient, QueryClientProvider } from "@tanstack/react-query";

const mocks = vi.hoisted(() => ({
  clusterProfileList: vi.fn(),
  clusterProfileActivate: vi.fn(),
  setAddr: vi.fn(),
}));

vi.mock("../../ipc/clusters", () => ({
  clusterProfileList: mocks.clusterProfileList,
  clusterProfileActivate: mocks.clusterProfileActivate,
  clusterProfileSave: vi.fn(),
  clusterProfileRemove: vi.fn(),
  clusterProfileMigrateLegacy: vi.fn(),
}));

vi.mock("../../ipc/connection", () => ({
  setAddr: mocks.setAddr,
  testConnection: vi.fn(),
  connState: vi.fn(),
  ping: vi.fn(),
  getDefaultAddr: vi.fn(),
}));

import { useConnection } from "../../state/connectionStore";
import { ClusterSelectorDropdown } from "../../components/ClusterSelectorDropdown";

function withClient(node: React.ReactNode) {
  const client = new QueryClient({
    defaultOptions: { queries: { retry: false, gcTime: 0 } },
  });
  return render(<QueryClientProvider client={client}>{node}</QueryClientProvider>);
}

beforeEach(() => {
  mocks.clusterProfileList.mockReset();
  mocks.clusterProfileActivate.mockReset();
  mocks.setAddr.mockReset();
  mocks.clusterProfileList.mockResolvedValue([]);
  mocks.clusterProfileActivate.mockResolvedValue({
    id: 1,
    label: "default",
    addr: "127.0.0.1:9999",
    tenant: 0,
    lastUsedAt: null,
    createdAt: 0,
  });
  mocks.setAddr.mockResolvedValue({ addr: "127.0.0.1:9999", status: { kind: "Connected" } });
  useConnection.setState({
    addr: "127.0.0.1:9999",
    status: { kind: "Connected" },
    hydrated: true,
  });
});

describe("<ClusterSelectorDropdown>", () => {
  it("opens the menu when the button is clicked and lists saved clusters", async () => {
    mocks.clusterProfileList.mockResolvedValueOnce([
      {
        id: 1,
        label: "alpha",
        addr: "127.0.0.1:9999",
        tenant: 0,
        lastUsedAt: null,
        createdAt: 0,
      },
      {
        id: 2,
        label: "beta",
        addr: "10.0.0.1:9999",
        tenant: 0,
        lastUsedAt: null,
        createdAt: 0,
      },
    ]);
    withClient(<ClusterSelectorDropdown onOpenSettings={() => {}} />);
    fireEvent.click(screen.getByRole("button", { name: /select cluster/i }));
    expect(await screen.findByRole("menuitem", { name: /alpha/i })).toBeInTheDocument();
    expect(screen.getByRole("menuitem", { name: /beta/i })).toBeInTheDocument();
  });

  it("activating a cluster calls clusterProfileActivate and setAddr", async () => {
    mocks.clusterProfileList.mockResolvedValueOnce([
      {
        id: 2,
        label: "beta",
        addr: "10.0.0.1:9999",
        tenant: 0,
        lastUsedAt: null,
        createdAt: 0,
      },
    ]);
    mocks.clusterProfileActivate.mockResolvedValueOnce({
      id: 2,
      label: "beta",
      addr: "10.0.0.1:9999",
      tenant: 0,
      lastUsedAt: null,
      createdAt: 0,
    });
    mocks.setAddr.mockResolvedValueOnce({
      addr: "10.0.0.1:9999",
      status: { kind: "Connected" },
    });
    withClient(<ClusterSelectorDropdown onOpenSettings={() => {}} />);
    fireEvent.click(screen.getByRole("button", { name: /select cluster/i }));
    const betaBtn = await screen.findByText("beta");
    fireEvent.click(betaBtn);
    await waitFor(() => expect(mocks.clusterProfileActivate).toHaveBeenCalledWith("10.0.0.1:9999"));
    expect(mocks.setAddr).toHaveBeenCalledWith("10.0.0.1:9999");
  });

  it("shows the active cluster address in the trigger label", () => {
    mocks.clusterProfileList.mockResolvedValueOnce([]);
    useConnection.setState({
      addr: "10.0.0.1:9999",
      status: { kind: "Connected" },
      hydrated: true,
    });
    withClient(<ClusterSelectorDropdown onOpenSettings={() => {}} />);
    expect(screen.getByText("10.0.0.1:9999")).toBeInTheDocument();
  });
});