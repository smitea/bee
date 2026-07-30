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
  pluginList as pluginListIpc,
  pluginSettingsGet as pluginSettingsGetIpc,
  pluginSettingsSet as pluginSettingsSetIpc,
  pluginScanDirectory as pluginScanDirectoryIpc,
  pluginDefaultDir as pluginDefaultDirIpc,
  pluginLastDir as pluginLastDirIpc,
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
    pluginList: vi.fn().mockResolvedValue([]),
    pluginScanDirectory: vi.fn().mockResolvedValue([]),
    pluginDefaultDir: vi.fn().mockResolvedValue("/Users/test/.bee/plugins"),
    pluginLastDir: vi.fn().mockResolvedValue(null),
    pluginSettingsGet: vi.fn().mockResolvedValue(null),
    pluginSettingsSet: vi.fn(),
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

vi.mock("../../ipc/plugin_settings", () => ({
  pluginSettingsGet: vi.fn().mockResolvedValue(null),
  pluginSettingsSet: vi.fn(),
}));

import { useConnection } from "../../state/connectionStore";
import { usePluginSettings } from "../../state/pluginSettingsStore";
import { SettingsModal } from "../../components/SettingsModal";

function withClient(node: React.ReactNode) {
  const client = new QueryClient({
    defaultOptions: { queries: { retry: false, gcTime: 0 } },
  });
  return render(<QueryClientProvider client={client}>{node}</QueryClientProvider>);
}

function resetPluginSettingsStore() {
  usePluginSettings.setState({
    hydrated: false,
    loading: false,
    enabled: {},
    config: {},
  });
}

beforeEach(() => {
  vi.mocked(setAddrIpc).mockReset();
  vi.mocked(testConnectionIpc).mockReset();
  vi.mocked(settingsGetIpc).mockReset();
  vi.mocked(settingsPutIpc).mockReset();
  vi.mocked(tenantGetIpc).mockReset();
  vi.mocked(tenantSetIpc).mockReset();
  vi.mocked(pluginListIpc).mockReset();
  vi.mocked(pluginSettingsGetIpc).mockReset();
  vi.mocked(pluginSettingsSetIpc).mockReset();
  vi.mocked(pluginScanDirectoryIpc).mockReset();
  vi.mocked(pluginDefaultDirIpc).mockReset();
  vi.mocked(pluginLastDirIpc).mockReset();
  vi.mocked(settingsGetIpc).mockResolvedValueOnce(null);
  vi.mocked(tenantGetIpc).mockResolvedValueOnce(0);
  vi.mocked(pluginListIpc).mockResolvedValue([]);
  vi.mocked(pluginScanDirectoryIpc).mockResolvedValue([]);
  vi.mocked(pluginDefaultDirIpc).mockResolvedValue("/Users/test/.bee/plugins");
  vi.mocked(pluginLastDirIpc).mockResolvedValue(null);
  vi.mocked(pluginSettingsGetIpc).mockResolvedValue(null);
  resetPluginSettingsStore();
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
  vi.mocked(pluginListIpc).mockReset();
  vi.mocked(pluginSettingsGetIpc).mockReset();
  vi.mocked(pluginSettingsSetIpc).mockReset();
  vi.mocked(pluginScanDirectoryIpc).mockReset();
  vi.mocked(pluginDefaultDirIpc).mockReset();
  vi.mocked(pluginLastDirIpc).mockReset();
  resetPluginSettingsStore();
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

function openPluginsSection() {
  fireEvent.click(screen.getByRole("button", { name: "Plugins" }));
}

describe("<SettingsModal> Plugins section", () => {
  it("renders an enable/disable toggle row for each loaded plugin with adapter chips", async () => {
    vi.mocked(pluginListIpc).mockResolvedValueOnce([
      { id: "p1", name: "binance", version: "1.4.2", adapters: ["subscribe"], handlers: [] },
      { id: "p2", name: "alpha", version: "0.9.0", adapters: [], handlers: ["compute"] },
    ]);
    withClient(<SettingsModal open onClose={() => {}} />);
    openPluginsSection();
    expect(await screen.findByTestId("plugin-row-binance")).toBeInTheDocument();
    expect(screen.getByTestId("plugin-row-alpha")).toBeInTheDocument();
    expect(screen.getAllByRole("button", { name: "Disable plugin" })).toHaveLength(2);
    expect(screen.getByText(/v1.4.2/)).toBeInTheDocument();
    expect(screen.getByText(/v0.9.0/)).toBeInTheDocument();
    expect(screen.getByTestId("plugin-toggle-all")).toBeInTheDocument();
  });

  it("clicking the plugin toggle calls pluginSettingsSet with the new enabled state", async () => {
    vi.mocked(pluginListIpc).mockResolvedValueOnce([
      { id: "p1", name: "binance", version: "1.0", adapters: ["subscribe"], handlers: [] },
    ]);
    vi.mocked(pluginSettingsGetIpc).mockResolvedValue({
      plugin_name: "binance",
      enabled: true,
      config_json: "{}",
      updated_at: 0,
    });
    vi.mocked(pluginSettingsSetIpc).mockResolvedValueOnce({
      plugin_name: "binance",
      enabled: false,
      config_json: "{}",
      updated_at: 1,
    });
    withClient(<SettingsModal open onClose={() => {}} />);
    openPluginsSection();
    await screen.findByTestId("plugin-row-binance");
    fireEvent.click(screen.getByRole("button", { name: /disable plugin/i }));
    await waitFor(() =>
      expect(pluginSettingsSetIpc).toHaveBeenCalledWith("binance", false, "{}"),
    );
  });

  it("shows a disabled badge for a plugin whose enabled state is false", async () => {
    vi.mocked(pluginListIpc).mockResolvedValueOnce([
      { id: "p1", name: "binance", version: "1.0", adapters: ["subscribe"], handlers: [] },
    ]);
    vi.mocked(pluginSettingsGetIpc).mockResolvedValueOnce({
      plugin_name: "binance",
      enabled: false,
      config_json: "{}",
      updated_at: 1,
    });
    withClient(<SettingsModal open onClose={() => {}} />);
    openPluginsSection();
    const row = await screen.findByTestId("plugin-row-binance");
    await screen.findByTestId("plugin-disabled-badge-binance");
    expect(row.getAttribute("data-enabled")).toBe("false");
    expect(screen.getByRole("button", { name: "Enable plugin" })).toBeInTheDocument();
  });
});
