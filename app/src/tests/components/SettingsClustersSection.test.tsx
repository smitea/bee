import { beforeEach, describe, expect, it, vi } from "vitest";
import { fireEvent, render, screen, waitFor } from "@testing-library/react";
import { QueryClient, QueryClientProvider } from "@tanstack/react-query";

const mocks = vi.hoisted(() => ({
  setAddr: vi.fn(),
  pluginList: vi.fn(),
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
  mocks.pluginSettingsGet.mockReset();
  mocks.pluginSettingsSet.mockReset();
  mocks.setAddr.mockResolvedValue({ addr: "127.0.0.1:9999", status: { kind: "Connected" } });
  mocks.pluginList.mockResolvedValue([]);
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
});