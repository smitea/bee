import { describe, it, expect, vi, beforeEach } from "vitest";

const applicationsList = vi.fn();
const applicationCreate = vi.fn();
const applicationSetEnabled = vi.fn();
const applicationDelete = vi.fn();

vi.mock("../../ipc/applications", () => ({
  applicationsList,
  applicationCreate,
  applicationSetEnabled,
  applicationDelete,
}));

beforeEach(() => {
  vi.resetModules();
  applicationsList.mockReset();
  applicationCreate.mockReset();
  applicationSetEnabled.mockReset();
  applicationDelete.mockReset();
});

function makeApp(id: number, name = "alpha", enabled = true) {
  return { id, name, enabled, display_order: id, created_at: 0 };
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