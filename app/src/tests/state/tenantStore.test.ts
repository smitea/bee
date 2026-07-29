import { describe, it, expect, vi, beforeEach } from "vitest";

const tenantGet = vi.fn();
const tenantSet = vi.fn();

vi.mock("../../ipc/tenant", () => ({
  tenantGet,
  tenantSet,
}));

beforeEach(() => {
  vi.resetModules();
  tenantGet.mockReset();
  tenantSet.mockReset();
});

describe("useTenant.refresh", () => {
  it("populates tenant from tenantGet", async () => {
    tenantGet.mockResolvedValueOnce(42);
    const { useTenant } = await import("../../state/tenantStore");
    await useTenant.getState().refresh();
    expect(useTenant.getState().tenant).toBe(42);
    expect(useTenant.getState().hydrated).toBe(true);
  });

  it("falls back to zero when tenantGet throws", async () => {
    tenantGet.mockRejectedValueOnce(new Error("boom"));
    const { useTenant } = await import("../../state/tenantStore");
    await useTenant.getState().refresh();
    expect(useTenant.getState().tenant).toBe(0);
    expect(useTenant.getState().hydrated).toBe(true);
  });
});

describe("useTenant.set", () => {
  it("calls tenantSet and updates the local tenant", async () => {
    tenantSet.mockResolvedValueOnce(7);
    const { useTenant } = await import("../../state/tenantStore");
    const result = await useTenant.getState().set(7);
    expect(result).toBe(7);
    expect(useTenant.getState().tenant).toBe(7);
  });
});