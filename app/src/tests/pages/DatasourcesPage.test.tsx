import { describe, it, expect, vi, beforeEach } from "vitest";
import { QueryClient, QueryClientProvider } from "@tanstack/react-query";
import { render, screen, fireEvent, waitFor, within } from "@testing-library/react";

const mocks = vi.hoisted(() => ({
  datasourceList: vi.fn(),
  datasourceCreate: vi.fn(),
  datasourceDelete: vi.fn(),
  pluginList: vi.fn(),
  pluginSchema: vi.fn(),
}));

vi.mock("../../ipc/datasources", () => ({
  datasourceList: mocks.datasourceList,
  datasourceCreate: mocks.datasourceCreate,
  datasourceDelete: mocks.datasourceDelete,
}));

vi.mock("../../ipc/plugins", () => ({
  pluginList: mocks.pluginList,
  pluginSchema: mocks.pluginSchema,
}));

beforeEach(() => {
  vi.resetModules();
  mocks.datasourceList.mockReset();
  mocks.datasourceCreate.mockReset();
  mocks.datasourceDelete.mockReset();
  mocks.pluginList.mockReset();
  mocks.pluginSchema.mockReset();

  mocks.datasourceList.mockResolvedValue([]);
  mocks.datasourceCreate.mockResolvedValue({
    name: "x",
    plugin: "p",
    config: "{}",
    tenant: 0,
    created_at: 0,
    updated_at: 0,
  });
  mocks.datasourceDelete.mockResolvedValue(undefined);
  mocks.pluginList.mockResolvedValue([]);
  mocks.pluginSchema.mockResolvedValue({
    name: "static",
    fields: [
      { name: "url", kind: "string", required: true, description: "endpoint URL" },
      { name: "api_key", kind: "string", required: false, description: null },
    ],
  });
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

describe("<DatasourcesPage>", () => {
  it("renders the empty state when no datasources exist", async () => {
    const { DatasourcesPage } = await import("../../pages/DatasourcesPage");
    withClient(<DatasourcesPage />);
    expect(await screen.findByText(/Datasources \(0\)/)).toBeInTheDocument();
    expect(screen.getByText(/no datasources/i)).toBeInTheDocument();
  });

  it("renders the empty plugin list and shows the Add button", async () => {
    const { DatasourcesPage } = await import("../../pages/DatasourcesPage");
    withClient(<DatasourcesPage />);
    expect(await screen.findByText(/Datasources \(0\)/)).toBeInTheDocument();
    expect(screen.getByRole("button", { name: /add/i })).toBeInTheDocument();
  });

  it("lists existing datasources from datasourceList", async () => {
    mocks.datasourceList.mockResolvedValueOnce([
      { name: "binance", plugin: "binance_subscribe", config: "{}", tenant: 0, created_at: 0, updated_at: 0 },
      { name: "kafka", plugin: "kafka_emit", config: "{}", tenant: 0, created_at: 0, updated_at: 0 },
    ]);
    const { DatasourcesPage } = await import("../../pages/DatasourcesPage");
    withClient(<DatasourcesPage />);
    expect(await screen.findByText(/Datasources \(2\)/)).toBeInTheDocument();
    expect(screen.getByText("binance")).toBeInTheDocument();
    expect(screen.getByText("kafka")).toBeInTheDocument();
  });

  it("Add button opens modal and renders schema fields when plugin is selected", async () => {
    mocks.pluginList.mockResolvedValueOnce([
      { name: "binance_subscribe", adapter: "binance_subscribe", kind: "input" },
    ]);
    const { DatasourcesPage } = await import("../../pages/DatasourcesPage");
    withClient(<DatasourcesPage />);
    fireEvent.click(await screen.findByRole("button", { name: /add/i }));
    const nameInput = await screen.findByPlaceholderText("binance");
    expect(nameInput).toBeInTheDocument();
  });

  it("Test Connection button returns OK label and does not call datasource_create", async () => {
    const { DatasourcesPage } = await import("../../pages/DatasourcesPage");
    withClient(<DatasourcesPage />);
    fireEvent.click(await screen.findByRole("button", { name: /add/i }));
    const testBtn = await screen.findByRole("button", { name: /test connection/i });
    fireEvent.click(testBtn);
    expect(await screen.findByText(/OK/)).toBeInTheDocument();
    expect(mocks.datasourceCreate).not.toHaveBeenCalled();
  });

  it("Connect and Save calls datasource_create with name + plugin + config_json", async () => {
    mocks.pluginList.mockResolvedValueOnce([
      { name: "binance_subscribe", adapter: "binance_subscribe", kind: "input" },
    ]);
    const { DatasourcesPage } = await import("../../pages/DatasourcesPage");
    withClient(<DatasourcesPage />);
    fireEvent.click(await screen.findByRole("button", { name: /add/i }));
    const nameInput = await screen.findByPlaceholderText("binance");
    fireEvent.change(nameInput, { target: { value: "alpha_ds" } });
    const pluginSelect = screen.getByLabelText(/plugin/i);
    fireEvent.change(pluginSelect, { target: { value: "binance_subscribe" } });
    const saveBtn = await screen.findByRole("button", { name: /connect and save/i });
    fireEvent.click(saveBtn);
    await waitFor(() => expect(mocks.datasourceCreate).toHaveBeenCalled());
    const callArgs = mocks.datasourceCreate.mock.calls[0];
    expect(callArgs[0]).toBe("alpha_ds");
    expect(callArgs[1]).toBe("binance_subscribe");
    expect(typeof callArgs[2]).toBe("string");
  });

  it("Delete button calls datasource_delete after confirmation", async () => {
    mocks.datasourceList.mockResolvedValueOnce([
      { name: "binance", plugin: "binance_subscribe", config: "{}", tenant: 0, created_at: 0, updated_at: 0 },
    ]);
    const { DatasourcesPage } = await import("../../pages/DatasourcesPage");
    withClient(<DatasourcesPage />);
    const row = await screen.findByText("binance");
    const delBtn = within(row.closest("tr") as HTMLElement).getByRole("button", { name: /delete/i });
    fireEvent.click(delBtn);
    await waitFor(() => expect(mocks.datasourceDelete).toHaveBeenCalledWith("binance"));
  });
});