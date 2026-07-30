import { describe, it, expect, vi, beforeEach } from "vitest";

const applicationsList = vi.fn();
const applicationCreate = vi.fn();
const applicationSetEnabled = vi.fn();
const applicationDelete = vi.fn();
const applicationEnable = vi.fn();
const applicationDisable = vi.fn();

vi.mock("../../ipc/applications", () => ({
  applicationsList,
  applicationCreate,
  applicationSetEnabled,
  applicationDelete,
  applicationEnable,
  applicationDisable,
}));

beforeEach(() => {
  vi.resetModules();
  applicationsList.mockReset();
  applicationCreate.mockReset();
  applicationSetEnabled.mockReset();
  applicationDelete.mockReset();
  applicationEnable.mockReset();
  applicationDisable.mockReset();
});

function makeApp(id: number, name = "alpha", enabled = true, tenant = 0) {
  return { id, name, enabled, display_order: id, tenant, created_at: 0 };
}

function makeEnableReport(appId: number) {
  return {
    application: makeApp(appId, "alpha", true),
    snapshot: null,
    succeeded: [{ kind: "pipeline", id: "p1" }],
    failed: [{ kind: "datasource", id: "binance", reason: "not connected" }],
    skipped: [],
    rehydrated: [
      { kind: "pipeline", name: "p1", result: "Success", detail: null },
      { kind: "datasource", name: "binance", result: "Failure", detail: "not connected" },
    ],
    outcome: "Degraded",
  };
}

function makeDisableReport(appId: number) {
  return {
    application: makeApp(appId, "alpha", false),
    snapshot: { application_id: appId, taken_at: 1234, payload_json: "{}" },
    succeeded: [
      { kind: "pipeline", id: "p1" },
      { kind: "datasource", id: "binance" },
    ],
    failed: [],
    skipped: [],
    pipelines: ["p1"],
    datasources: ["binance"],
    outcome: "Success",
  };
}

describe("useApplications.refresh", () => {
  it("populates items from applicationsList", async () => {
    applicationsList.mockResolvedValueOnce([makeApp(1), makeApp(2)]);
    const { useApplications } = await import("../../state/applicationsStore");
    await useApplications.getState().refresh();
    expect(useApplications.getState().items.map((a) => a.id)).toEqual([1, 2]);
    expect(useApplications.getState().loaded).toBe(true);
  });

  it("create appends a new item", async () => {
    applicationsList.mockResolvedValueOnce([]);
    applicationCreate.mockResolvedValueOnce(makeApp(7, "beta"));
    const { useApplications } = await import("../../state/applicationsStore");
    await useApplications.getState().refresh();
    const created = await useApplications.getState().create("beta");
    expect(created.id).toBe(7);
    expect(useApplications.getState().items.map((a) => a.id)).toEqual([7]);
  });

  it("create with tenant passes through", async () => {
    applicationsList.mockResolvedValueOnce([]);
    applicationCreate.mockResolvedValueOnce(makeApp(7, "beta", true, 5));
    const { useApplications } = await import("../../state/applicationsStore");
    await useApplications.getState().refresh();
    const created = await useApplications.getState().create("beta", 5);
    expect(created.tenant).toBe(5);
    expect(applicationCreate).toHaveBeenCalledWith("beta", 5);
  });

  it("setEnabled toggles the local flag without refetching", async () => {
    applicationsList.mockResolvedValueOnce([makeApp(1, "alpha", true)]);
    applicationSetEnabled.mockResolvedValueOnce(undefined);
    const { useApplications } = await import("../../state/applicationsStore");
    await useApplications.getState().refresh();
    await useApplications.getState().setEnabled(1, false);
    expect(useApplications.getState().items[0].enabled).toBe(false);
    expect(applicationsList).toHaveBeenCalledTimes(1);
  });

  it("delete removes the item", async () => {
    applicationsList.mockResolvedValueOnce([makeApp(1), makeApp(2)]);
    applicationDelete.mockResolvedValueOnce(undefined);
    const { useApplications } = await import("../../state/applicationsStore");
    await useApplications.getState().refresh();
    await useApplications.getState().delete(1);
    expect(useApplications.getState().items.map((a) => a.id)).toEqual([2]);
  });
});

describe("useApplications.enable / disable", () => {
  it("enable returns the EnableReport shape with succeeded/failed/skipped/outcome", async () => {
    applicationsList.mockResolvedValueOnce([makeApp(1, "alpha", false)]);
    const report = makeEnableReport(1);
    applicationEnable.mockResolvedValueOnce(report);

    const { useApplications } = await import("../../state/applicationsStore");
    await useApplications.getState().refresh();
    const out = await useApplications.getState().enable(1);

    expect(out.application.id).toBe(1);
    expect(out.outcome).toBe("Degraded");
    expect(out.succeeded).toEqual([{ kind: "pipeline", id: "p1" }]);
    expect(out.failed).toEqual([
      { kind: "datasource", id: "binance", reason: "not connected" },
    ]);
    expect(out.skipped).toEqual([]);
    expect(out.rehydrated).toHaveLength(2);
    expect(useApplications.getState().items[0].enabled).toBe(true);
    expect(applicationEnable).toHaveBeenCalledWith(1);
  });

  it("enable is callable without an addr argument", async () => {
    applicationsList.mockResolvedValueOnce([makeApp(1, "alpha", false)]);
    applicationEnable.mockResolvedValueOnce(makeEnableReport(1));

    const { useApplications } = await import("../../state/applicationsStore");
    await useApplications.getState().refresh();
    const out = await useApplications.getState().enable(1);
    expect(out.outcome).toBe("Degraded");
    expect(applicationEnable).toHaveBeenCalledWith(1);
  });

  it("disable returns the DisableReport shape with succeeded/failed/skipped/outcome", async () => {
    applicationsList.mockResolvedValueOnce([makeApp(1, "alpha", true)]);
    const report = makeDisableReport(1);
    applicationDisable.mockResolvedValueOnce(report);

    const { useApplications } = await import("../../state/applicationsStore");
    await useApplications.getState().refresh();
    const out = await useApplications.getState().disable(1);

    expect(out.application.id).toBe(1);
    expect(out.outcome).toBe("Success");
    expect(out.succeeded).toEqual([
      { kind: "pipeline", id: "p1" },
      { kind: "datasource", id: "binance" },
    ]);
    expect(out.failed).toEqual([]);
    expect(out.skipped).toEqual([]);
    expect(out.pipelines).toEqual(["p1"]);
    expect(out.datasources).toEqual(["binance"]);
    expect(useApplications.getState().items[0].enabled).toBe(false);
  });
});