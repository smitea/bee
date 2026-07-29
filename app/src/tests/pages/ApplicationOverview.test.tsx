import { describe, it, expect, vi, beforeEach } from "vitest";
import { render, screen, fireEvent, waitFor } from "@testing-library/react";

const applicationsList = vi.fn();
const applicationCreate = vi.fn();
const applicationSetEnabled = vi.fn();
const applicationEnable = vi.fn();
const applicationDisable = vi.fn();
const applicationDelete = vi.fn();
const auditQuery = vi.fn();
const applicationExport = vi.fn();
const applicationImport = vi.fn();

vi.mock("../../ipc/applications", () => ({
  applicationsList,
  applicationCreate,
  applicationSetEnabled,
  applicationEnable,
  applicationDisable,
  applicationDelete,
  applicationExport,
  applicationImport,
}));

vi.mock("../../ipc/audit", () => ({
  auditQuery,
}));

beforeEach(() => {
  vi.resetModules();
  applicationsList.mockReset();
  applicationCreate.mockReset();
  applicationSetEnabled.mockReset();
  applicationEnable.mockReset();
  applicationDisable.mockReset();
  applicationDelete.mockReset();
  applicationExport.mockReset();
  applicationImport.mockReset();
  auditQuery.mockReset();

  applicationsList.mockResolvedValue([
    { id: 1, name: "alpha", enabled: true, display_order: 1, created_at: 0 },
  ]);
  auditQuery.mockResolvedValue([]);
  applicationExport.mockResolvedValue(undefined);
  applicationImport.mockResolvedValue({ created: ["alpha"], skipped: [] });
  applicationEnable.mockImplementation(async (id: number) => ({
    id,
    name: "alpha",
    enabled: true,
    display_order: 1,
    created_at: 0,
  }));
  applicationDisable.mockImplementation(async (id: number) => ({
    application: {
      id,
      name: "alpha",
      enabled: false,
      display_order: 1,
      created_at: 0,
    },
    snapshot: {
      application_id: id,
      taken_at: 0,
      payload_json: "{}",
    },
    pipelines: [],
    datasources: [],
  }));
});

describe("<ApplicationOverview> import/export", () => {
  it("renders Export and Import sections", async () => {
    const { ApplicationOverview } = await import("../../pages/ApplicationOverview");
    render(<ApplicationOverview applicationId={1} />);
    expect(await screen.findAllByText(/^Export$/)).toHaveLength(2);
    expect(screen.getAllByText(/^Import$/)).toHaveLength(2);
    expect(screen.getAllByText(/Passphrase/).length).toBeGreaterThanOrEqual(1);
    expect(screen.getAllByText(/File path/).length).toBeGreaterThanOrEqual(2);
  });

  it("Export button calls application_export with the typed passphrase and path", async () => {
    const { ApplicationOverview } = await import("../../pages/ApplicationOverview");
    render(<ApplicationOverview applicationId={1} />);
    const passInputs = await screen.findAllByLabelText(/passphrase/i);
    const pathInputs = await screen.findAllByLabelText(/file path/i);
    const exportPass = passInputs[0];
    const exportPath = pathInputs[0];
    fireEvent.change(exportPass, { target: { value: "secret1" } });
    fireEvent.change(exportPath, { target: { value: "/tmp/a.bapp" } });
    const exportBtn = screen.getByRole("button", { name: /^export$/i });
    fireEvent.click(exportBtn);
    await waitFor(() => expect(applicationExport).toHaveBeenCalled());
    const args = applicationExport.mock.calls[0];
    expect(args[0]).toBe("alpha");
    expect(args[1]).toBe("secret1");
    expect(args[2]).toBe("/tmp/a.bapp");
  });

  it("Import button calls application_import with the typed passphrase and path", async () => {
    const { ApplicationOverview } = await import("../../pages/ApplicationOverview");
    render(<ApplicationOverview applicationId={1} />);
    const passInputs = await screen.findAllByLabelText(/passphrase/i);
    const pathInputs = await screen.findAllByLabelText(/file path/i);
    const importPass = passInputs[1];
    const importPath = pathInputs[1];
    fireEvent.change(importPass, { target: { value: "secret2" } });
    fireEvent.change(importPath, { target: { value: "/tmp/in.bapp" } });
    const importBtn = screen.getByRole("button", { name: /^import$/i });
    fireEvent.click(importBtn);
    await waitFor(() => expect(applicationImport).toHaveBeenCalled());
    const args = applicationImport.mock.calls[0];
    expect(args[0]).toBe("/tmp/in.bapp");
    expect(args[1]).toBe("secret2");
  });

  it("Import button surfaces an error message when the import fails", async () => {
    applicationImport.mockRejectedValueOnce(new Error("decryption failed"));
    const { ApplicationOverview } = await import("../../pages/ApplicationOverview");
    render(<ApplicationOverview applicationId={1} />);
    const passInputs = await screen.findAllByLabelText(/passphrase/i);
    const pathInputs = await screen.findAllByLabelText(/file path/i);
    fireEvent.change(passInputs[1], { target: { value: "x" } });
    fireEvent.change(pathInputs[1], { target: { value: "/tmp/x.bapp" } });
    fireEvent.click(screen.getByRole("button", { name: /^import$/i }));
    const msg = await screen.findByText(/decryption failed/);
    expect(msg).toBeInTheDocument();
  });

  it("Export button surfaces a success message after a successful export", async () => {
    const { ApplicationOverview } = await import("../../pages/ApplicationOverview");
    render(<ApplicationOverview applicationId={1} />);
    const passInputs = await screen.findAllByLabelText(/passphrase/i);
    const pathInputs = await screen.findAllByLabelText(/file path/i);
    fireEvent.change(passInputs[0], { target: { value: "secret1" } });
    fireEvent.change(pathInputs[0], { target: { value: "/tmp/a.bapp" } });
    fireEvent.click(screen.getByRole("button", { name: /^export$/i }));
    expect(await screen.findByText(/exported/i)).toBeInTheDocument();
  });

  it("Import success summary shows created/skipped counts", async () => {
    applicationImport.mockResolvedValueOnce({
      created: ["alpha"],
      skipped: ["beta"],
    });
    const { ApplicationOverview } = await import("../../pages/ApplicationOverview");
    render(<ApplicationOverview applicationId={1} />);
    const passInputs = await screen.findAllByLabelText(/passphrase/i);
    const pathInputs = await screen.findAllByLabelText(/file path/i);
    fireEvent.change(passInputs[1], { target: { value: "x" } });
    fireEvent.change(pathInputs[1], { target: { value: "/tmp/in.bapp" } });
    fireEvent.click(screen.getByRole("button", { name: /^import$/i }));
    const summary = await screen.findByTestId("import-summary");
    expect(summary.textContent).toMatch(/alpha/);
    expect(summary.textContent).toMatch(/beta/);
  });

  it("Audit list filters by the current application id", async () => {
    const { ApplicationOverview } = await import("../../pages/ApplicationOverview");
    render(<ApplicationOverview applicationId={1} />);
    await waitFor(() => expect(auditQuery).toHaveBeenCalledWith(1, 25));
  });

  it("Disable button calls application_disable and shows snapshot summary", async () => {
    applicationDisable.mockResolvedValueOnce({
      application: {
        id: 1,
        name: "alpha",
        enabled: false,
        display_order: 1,
        created_at: 0,
      },
      snapshot: {
        application_id: 1,
        taken_at: 12345,
        payload_json: '{"pipelines":[],"datasources":[]}',
      },
      pipelines: ["p1", "p2"],
      datasources: ["binance"],
    });
    const { ApplicationOverview } = await import("../../pages/ApplicationOverview");
    render(<ApplicationOverview applicationId={1} />);
    const disableBtn = await screen.findByTestId("disable-app");
    fireEvent.click(disableBtn);
    await waitFor(() => expect(applicationDisable).toHaveBeenCalledWith(1));
    const summary = await screen.findByTestId("disable-summary");
    expect(summary.textContent).toMatch(/pipelines: 2/);
    expect(summary.textContent).toMatch(/datasources: 1/);
  });

  it("Enable button calls application_enable and shows success indicator", async () => {
    applicationsList.mockResolvedValueOnce([
      { id: 1, name: "alpha", enabled: false, display_order: 1, created_at: 0 },
    ]);
    applicationEnable.mockResolvedValueOnce({
      id: 1,
      name: "alpha",
      enabled: true,
      display_order: 1,
      created_at: 0,
    });
    const { ApplicationOverview } = await import("../../pages/ApplicationOverview");
    render(<ApplicationOverview applicationId={1} />);
    const enableBtn = await screen.findByTestId("enable-app");
    fireEvent.click(enableBtn);
    await waitFor(() => expect(applicationEnable).toHaveBeenCalledWith(1));
    expect(await screen.findByText(/enabled/i)).toBeInTheDocument();
  });
});