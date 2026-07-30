import { beforeEach, describe, expect, it, vi } from "vitest";
import { fireEvent, render, screen, waitFor } from "@testing-library/react";
import { QueryClient, QueryClientProvider } from "@tanstack/react-query";

const mocks = vi.hoisted(() => ({
  clusterProfileList: vi.fn(),
  clusterProfileSave: vi.fn(),
  clusterProfileRemove: vi.fn(),
  clusterProfileActivate: vi.fn(),
  clusterProfileMigrateLegacy: vi.fn(),
  setAddr: vi.fn(),
}));

vi.mock("../../ipc/clusters", () => ({
  clusterProfileList: mocks.clusterProfileList,
  clusterProfileSave: mocks.clusterProfileSave,
  clusterProfileRemove: mocks.clusterProfileRemove,
  clusterProfileActivate: mocks.clusterProfileActivate,
  clusterProfileMigrateLegacy: mocks.clusterProfileMigrateLegacy,
}));

vi.mock("../../ipc/connection", () => ({
  setAddr: mocks.setAddr,
  testConnection: vi.fn(),
  connState: vi.fn(),
  ping: vi.fn(),
  getDefaultAddr: vi.fn(),
}));

vi.mock("../../ipc/settings", () => ({
  settingsGet: vi.fn().mockResolvedValue(null),
  settingsPut: vi.fn().mockResolvedValue(undefined),
}));

vi.mock("../../ipc/tenant", () => ({
  tenantGet: vi.fn().mockResolvedValue(0),
  tenantSet: vi.fn().mockResolvedValue(0),
}));

import { SettingsModal } from "../../components/SettingsModal";
import { useConnection } from "../../state/connectionStore";

function withClient(node: React.ReactNode) {
  const client = new QueryClient({
    defaultOptions: { queries: { retry: false, gcTime: 0 } },
  });
  return render(<QueryClientProvider client={client}>{node}</QueryClientProvider>);
}

beforeEach(() => {
  mocks.clusterProfileList.mockReset();
  mocks.clusterProfileSave.mockReset();
  mocks.clusterProfileRemove.mockReset();
  mocks.clusterProfileActivate.mockReset();
  mocks.clusterProfileMigrateLegacy.mockReset();
  mocks.setAddr.mockReset();
  mocks.clusterProfileList.mockResolvedValue([]);
  mocks.clusterProfileMigrateLegacy.mockResolvedValue({ inserted: 0, skipped: [] });
  mocks.setAddr.mockResolvedValue({ addr: "127.0.0.1:9999", status: { kind: "Connected" } });
  useConnection.setState({
    addr: "127.0.0.1:9999",
    status: { kind: "Connected" },
    hydrated: true,
  });
});

describe("<SettingsModal> Clusters section", () => {
  it("lists saved clusters with status dots and Connect / Edit / Remove controls under Connection", async () => {
    mocks.clusterProfileList.mockResolvedValue([
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
    withClient(<SettingsModal open onClose={() => {}} />);
    expect(await screen.findByText("alpha")).toBeInTheDocument();
    expect(screen.getByText("beta")).toBeInTheDocument();
    expect(screen.getByRole("button", { name: /connect alpha/i })).toBeInTheDocument();
    expect(screen.getByRole("button", { name: /edit alpha/i })).toBeInTheDocument();
    expect(screen.getByRole("button", { name: /remove alpha/i })).toBeInTheDocument();
    const dots = screen.getAllByRole("status");
    expect(dots.length).toBeGreaterThan(0);
    expect(dots.some((d) => d.getAttribute("data-status") === "green")).toBe(true);
    expect(dots.some((d) => d.getAttribute("data-status") === "amber")).toBe(true);
  });

  it("add cluster invokes clusterProfileSave", async () => {
    mocks.clusterProfileList.mockResolvedValue([]);
    mocks.clusterProfileSave.mockResolvedValueOnce(99);
    withClient(<SettingsModal open onClose={() => {}} />);
    fireEvent.click(await screen.findByRole("button", { name: /add cluster/i }));
    fireEvent.change(screen.getByLabelText("Cluster label"), {
      target: { value: "new" },
    });
    fireEvent.change(screen.getByLabelText("Cluster address"), {
      target: { value: "1.2.3.4:9999" },
    });
    fireEvent.click(screen.getByRole("button", { name: /^save$/i }));
    await waitFor(() => expect(mocks.clusterProfileSave).toHaveBeenCalledWith("new", "1.2.3.4:9999", 0));
  });

  it("remove cluster invokes clusterProfileRemove", async () => {
    mocks.clusterProfileList.mockResolvedValue([
      {
        id: 5,
        label: "del",
        addr: "10.0.0.5:9999",
        tenant: 0,
        lastUsedAt: null,
        createdAt: 0,
      },
    ]);
    mocks.clusterProfileRemove.mockResolvedValueOnce(undefined);
    withClient(<SettingsModal open onClose={() => {}} />);
    fireEvent.click(await screen.findByRole("button", { name: /remove del/i }));
    await waitFor(() => expect(mocks.clusterProfileRemove).toHaveBeenCalledWith("10.0.0.5:9999"));
  });
});