import { beforeEach, describe, expect, it, vi } from "vitest";
import { fireEvent, render, screen, waitFor } from "@testing-library/react";
import { QueryClient, QueryClientProvider } from "@tanstack/react-query";

const mocks = vi.hoisted(() => ({
  setAddr: vi.fn(),
  pluginList: vi.fn(),
  pluginScanDirectory: vi.fn(),
  pluginDefaultDir: vi.fn(),
  pluginLastDir: vi.fn(),
  pluginSettingsGet: vi.fn(),
  pluginSettingsSet: vi.fn(),
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

vi.mock("../../ipc/plugins", () => ({
  pluginList: mocks.pluginList,
  pluginScanDirectory: mocks.pluginScanDirectory,
  pluginDefaultDir: mocks.pluginDefaultDir,
  pluginLastDir: mocks.pluginLastDir,
  pluginSettingsGet: mocks.pluginSettingsGet,
  pluginSettingsSet: mocks.pluginSettingsSet,
}));

vi.mock("../../ipc/plugin_settings", () => ({
  pluginSettingsGet: mocks.pluginSettingsGet,
  pluginSettingsSet: mocks.pluginSettingsSet,
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
  mocks.setAddr.mockReset();
  mocks.pluginList.mockReset();
  mocks.pluginScanDirectory.mockReset();
  mocks.pluginDefaultDir.mockReset();
  mocks.pluginLastDir.mockReset();
  mocks.pluginSettingsGet.mockReset();
  mocks.pluginSettingsSet.mockReset();
  mocks.setAddr.mockResolvedValue({ addr: "127.0.0.1:9999", status: { kind: "Connected" } });
  mocks.pluginList.mockResolvedValue([]);
  mocks.pluginScanDirectory.mockResolvedValue([]);
  mocks.pluginDefaultDir.mockResolvedValue("/Users/test/.bee/plugins");
  mocks.pluginLastDir.mockResolvedValue(null);
  mocks.pluginSettingsGet.mockResolvedValue(null);
  useConnection.setState({
    addr: "127.0.0.1:9999",
    status: { kind: "Connected" },
    hydrated: true,
  });
});

describe("<SettingsModal> Connection section", () => {
  it("renders a single connection editor (label + addr) with Test/Connect buttons", async () => {
    withClient(<SettingsModal open onClose={() => {}} />);
    expect(await screen.findByLabelText("AdminServer address")).toBeInTheDocument();
    expect(screen.getByLabelText("Connection label")).toBeInTheDocument();
    expect(screen.getByRole("button", { name: "Test Connection" })).toBeInTheDocument();
    expect(screen.getByRole("button", { name: "Connect" })).toBeInTheDocument();
  });
});

describe("<SettingsModal> Plugins section", () => {
  it("shows loaded plugins from plugin_list with enable toggle + config schema preview", async () => {
    mocks.pluginList.mockResolvedValue([
      {
        id: "p1",
        name: "binance",
        version: "1.4.2",
        adapters: ["subscribe"],
        handlers: [],
      },
    ]);
    mocks.pluginSettingsGet.mockResolvedValue({
      plugin_name: "binance",
      enabled: true,
      config_json: "{}",
      updated_at: 0,
    });
    withClient(<SettingsModal open onClose={() => {}} />);
    fireEvent.click(screen.getByRole("button", { name: "Plugins" }));
    expect(await screen.findByTestId("plugins-count")).toBeInTheDocument();
    expect(await screen.findByTestId("plugin-row-binance")).toBeInTheDocument();
    expect(screen.getByText(/v1.4.2/)).toBeInTheDocument();
  });

  it("invokes pluginSettingsSet when toggling enabled", async () => {
    mocks.pluginList.mockResolvedValue([
      { id: "p1", name: "binance", version: "1.0", adapters: ["subscribe"], handlers: [] },
    ]);
    mocks.pluginSettingsGet.mockResolvedValue({
      plugin_name: "binance",
      enabled: true,
      config_json: "{}",
      updated_at: 0,
    });
    mocks.pluginSettingsSet.mockResolvedValueOnce({
      plugin_name: "binance",
      enabled: false,
      config_json: "{}",
      updated_at: 1,
    });
    withClient(<SettingsModal open onClose={() => {}} />);
    fireEvent.click(screen.getByRole("button", { name: "Plugins" }));
    await screen.findByTestId("plugin-row-binance");
    fireEvent.click(screen.getByRole("button", { name: /disable plugin/i }));
    await waitFor(() =>
      expect(mocks.pluginSettingsSet).toHaveBeenCalledWith("binance", false, "{}"),
    );
  });

  it("shows empty state when no plugins are loaded", async () => {
    mocks.pluginList.mockResolvedValue([]);
    withClient(<SettingsModal open onClose={() => {}} />);
    fireEvent.click(screen.getByRole("button", { name: "Plugins" }));
    expect(await screen.findByText(/no plugins loaded/i)).toBeInTheDocument();
  });

  it("renders a Reload from disk button with the resolved plugin directory", async () => {
    withClient(<SettingsModal open onClose={() => {}} />);
    fireEvent.click(screen.getByRole("button", { name: "Plugins" }));
    const input = (await screen.findByTestId("plugin-dir-input")) as HTMLInputElement;
    await waitFor(() => expect(input.value).toBe("/Users/test/.bee/plugins"));
    expect(screen.getByTestId("plugin-reload")).toBeInTheDocument();
    expect(screen.getByTestId("plugin-scan-status")).toBeInTheDocument();
  });

  it("falls back to the default directory when the persisted last dir is empty", async () => {
    mocks.pluginLastDir.mockResolvedValue(null);
    withClient(<SettingsModal open onClose={() => {}} />);
    fireEvent.click(screen.getByRole("button", { name: "Plugins" }));
    const input = (await screen.findByTestId("plugin-dir-input")) as HTMLInputElement;
    await waitFor(() => expect(input.value).toBe("/Users/test/.bee/plugins"));
  });

  it("uses the last-scanned directory persisted in client_settings when available", async () => {
    mocks.pluginLastDir.mockResolvedValue("/opt/bee/plugins");
    withClient(<SettingsModal open onClose={() => {}} />);
    fireEvent.click(screen.getByRole("button", { name: "Plugins" }));
    const input = (await screen.findByTestId("plugin-dir-input")) as HTMLInputElement;
    await waitFor(() => expect(input.value).toBe("/opt/bee/plugins"));
  });

  it("invokes pluginScanDirectory with the resolved path when Reload from disk is clicked", async () => {
    mocks.pluginScanDirectory.mockResolvedValueOnce([]);
    withClient(<SettingsModal open onClose={() => {}} />);
    fireEvent.click(screen.getByRole("button", { name: "Plugins" }));
    const input = (await screen.findByTestId("plugin-dir-input")) as HTMLInputElement;
    await waitFor(() => expect(input.value).toBe("/Users/test/.bee/plugins"));
    fireEvent.click(screen.getByTestId("plugin-reload"));
    await waitFor(() =>
      expect(mocks.pluginScanDirectory).toHaveBeenCalledWith("/Users/test/.bee/plugins"),
    );
  });

  it("scans with a custom path typed into the plugin directory input", async () => {
    mocks.pluginScanDirectory.mockResolvedValueOnce([]);
    withClient(<SettingsModal open onClose={() => {}} />);
    fireEvent.click(screen.getByRole("button", { name: "Plugins" }));
    const input = (await screen.findByTestId("plugin-dir-input")) as HTMLInputElement;
    await waitFor(() => expect(input.value).toBe("/Users/test/.bee/plugins"));
    fireEvent.change(input, { target: { value: "/tmp/my-bee-plugins" } });
    fireEvent.click(screen.getByTestId("plugin-reload"));
    await waitFor(() =>
      expect(mocks.pluginScanDirectory).toHaveBeenCalledWith("/tmp/my-bee-plugins"),
    );
  });
});