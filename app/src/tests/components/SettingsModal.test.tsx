import { afterEach, beforeEach, describe, expect, it, vi } from "vitest";
import { fireEvent, render, screen, waitFor } from "@testing-library/react";
import { QueryClient, QueryClientProvider } from "@tanstack/react-query";

import {
  setAddr as setAddrIpc,
  testConnection as testConnectionIpc,
  settingsGet as settingsGetIpc,
  settingsPut as settingsPutIpc,
  tenantGet as tenantGetIpc,
  tenantSet as tenantSetIpc,
} from "../../ipc";

vi.mock("../../ipc", async () => {
  const actual = await vi.importActual<typeof import("../../ipc")>("../../ipc");
  return {
    ...actual,
    setAddr: vi.fn(),
    testConnection: vi.fn(),
    settingsGet: vi.fn(),
    settingsPut: vi.fn(),
    tenantGet: vi.fn(),
    tenantSet: vi.fn(),
    pluginScanDirectory: vi.fn().mockResolvedValue([]),
    pluginDefaultDir: vi.fn().mockResolvedValue("/Users/test/.bee/plugins"),
    pluginLastDir: vi.fn().mockResolvedValue(null),
  };
});

vi.mock("../../ipc/clusters", () => ({
  clusterProfileList: vi.fn().mockResolvedValue([]),
  clusterProfileSave: vi.fn(),
  clusterProfileRemove: vi.fn(),
  clusterProfileActivate: vi.fn().mockResolvedValue({
    id: 1,
    label: "default",
    addr: "127.0.0.1:9999",
    tenant: 0,
    lastUsedAt: null,
    createdAt: 0,
  }),
  clusterProfileMigrateLegacy: vi.fn(),
}));

import { useConnection } from "../../state/connectionStore";
import { SettingsModal } from "../../components/SettingsModal";

function withClient(node: React.ReactNode) {
  const client = new QueryClient({
    defaultOptions: { queries: { retry: false, gcTime: 0 } },
  });
  return render(<QueryClientProvider client={client}>{node}</QueryClientProvider>);
}

beforeEach(() => {
  vi.mocked(setAddrIpc).mockReset();
  vi.mocked(testConnectionIpc).mockReset();
  vi.mocked(settingsGetIpc).mockReset();
  vi.mocked(settingsPutIpc).mockReset();
  vi.mocked(tenantGetIpc).mockReset();
  vi.mocked(tenantSetIpc).mockReset();
  vi.mocked(settingsGetIpc).mockResolvedValueOnce(null);
  vi.mocked(tenantGetIpc).mockResolvedValueOnce(0);
  useConnection.setState({
    addr: "127.0.0.1:9999",
    status: { kind: "Connecting" },
    hydrated: false,
  });
});

afterEach(() => {
  vi.mocked(setAddrIpc).mockReset();
  vi.mocked(testConnectionIpc).mockReset();
  vi.mocked(settingsGetIpc).mockReset();
  vi.mocked(settingsPutIpc).mockReset();
  vi.mocked(tenantGetIpc).mockReset();
  vi.mocked(tenantSetIpc).mockReset();
});

function openTenantSection() {
  fireEvent.click(screen.getByRole("button", { name: "Tenant" }));
}

describe("<SettingsModal>", () => {
  it("renders nothing when closed", () => {
    const { container } = withClient(<SettingsModal open={false} onClose={() => {}} />);
    expect(container.firstChild).toBeNull();
  });

  it("lists every settings category in the sidebar (11 merged entries)", () => {
    withClient(<SettingsModal open onClose={() => {}} />);
    for (const label of [
      "Client",
      "Connection",
      "Tenant",
      "Appearance",
      "Logging",
      "Diagnostics",
      "Raft",
      "KV",
      "Scheduling",
      "Plugins",
      "Security",
    ]) {
      expect(screen.getByRole("button", { name: label })).toBeInTheDocument();
    }
    expect(screen.queryByRole("button", { name: "Cluster" })).toBeNull();
  });

  it("Connection section is the default and shows single connection editor (label + address + Test/Connect)", () => {
    withClient(<SettingsModal open onClose={() => {}} />);
    expect(screen.getByLabelText("AdminServer address")).toBeInTheDocument();
    expect(screen.getByLabelText("Connection label")).toBeInTheDocument();
    expect(screen.getByRole("button", { name: "Test Connection" })).toBeInTheDocument();
    expect(screen.getByRole("button", { name: "Connect" })).toBeInTheDocument();
  });

  it("Test Connection does not change the active address", async () => {
    vi.mocked(testConnectionIpc).mockResolvedValueOnce({
      addr: "10.0.0.1:9999",
      status: { kind: "Error", reason: "refused" },
    });
    withClient(<SettingsModal open onClose={() => {}} />);
    fireEvent.change(screen.getByLabelText("AdminServer address"), {
      target: { value: "10.0.0.1:9999" },
    });
    fireEvent.click(screen.getByText("Test Connection"));
    await waitFor(() => expect(testConnectionIpc).toHaveBeenCalled());
    expect(setAddrIpc).not.toHaveBeenCalled();
  });

  it("Connect switches the active connection and closes", async () => {
    vi.mocked(setAddrIpc).mockResolvedValueOnce({
      addr: "127.0.0.1:9999",
      status: { kind: "Connected" },
    });
    const onClose = vi.fn();
    withClient(<SettingsModal open onClose={onClose} />);
    fireEvent.click(screen.getByText("Connect"));
    await waitFor(() => expect(setAddrIpc).toHaveBeenCalledWith("127.0.0.1:9999"));
    expect(onClose).toHaveBeenCalled();
  });

  it("shows the active tenant loaded from tenantGet", async () => {
    vi.mocked(tenantGetIpc).mockReset();
    vi.mocked(tenantGetIpc).mockResolvedValue(7);
    withClient(<SettingsModal open onClose={() => {}} />);
    openTenantSection();
    await waitFor(() => expect(screen.getByLabelText("Active tenant")).toBeInTheDocument());
    expect((screen.getByLabelText("Active tenant") as HTMLInputElement).value).toBe("7");
  });

  it("changing the tenant field calls tenant_set with the typed value", async () => {
    vi.mocked(tenantGetIpc).mockReset();
    vi.mocked(tenantGetIpc).mockResolvedValue(0);
    vi.mocked(tenantSetIpc).mockReset();
    vi.mocked(tenantSetIpc).mockResolvedValue(42);
    withClient(<SettingsModal open onClose={() => {}} />);
    openTenantSection();
    const input = await screen.findByLabelText("Active tenant");
    fireEvent.change(input, { target: { value: "42" } });
    await waitFor(() => expect(tenantSetIpc).toHaveBeenCalledWith(42));
  });

  it("rejects an out-of-range tenant and shows an error", async () => {
    withClient(<SettingsModal open onClose={() => {}} />);
    openTenantSection();
    const input = await screen.findByLabelText("Active tenant");
    fireEvent.change(input, { target: { value: "70000" } });
    const err = await screen.findByText(/tenant must be a number between 0 and 65535/);
    expect(err).toBeInTheDocument();
    expect(tenantSetIpc).not.toHaveBeenCalled();
  });
});
