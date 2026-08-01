import { describe, it, expect, vi, beforeEach } from "vitest";

const mocks = vi.hoisted(() => ({
  invoke: vi.fn(),
}));

vi.mock("@tauri-apps/api/core", () => ({
  invoke: mocks.invoke,
}));

beforeEach(() => {
  mocks.invoke.mockReset();
  mocks.invoke.mockResolvedValue(undefined);
});

describe("ipc/tabs", () => {
  it("tabOpen sends camelCase resourceId to match Tauri 2's default #[tauri::command] arg rename", async () => {
    const { tabOpen } = await import("../../ipc/tabs");
    mocks.invoke.mockResolvedValueOnce(42);
    const id = await tabOpen("pipeline", "7", "alpha");
    expect(id).toBe(42);
    expect(mocks.invoke).toHaveBeenCalledTimes(1);
    const [cmd, payload] = mocks.invoke.mock.calls[0];
    expect(cmd).toBe("tab_open");
    expect(payload).toEqual({ kind: "pipeline", resourceId: "7", title: "alpha" });
    expect(Object.prototype.hasOwnProperty.call(payload, "resource_id")).toBe(false);
  });

  it("tabOpen forwards null resourceId as camelCase null", async () => {
    const { tabOpen } = await import("../../ipc/tabs");
    mocks.invoke.mockResolvedValueOnce(1);
    await tabOpen("cluster", null, "Cluster");
    const [, payload] = mocks.invoke.mock.calls[0];
    expect(payload).toEqual({ kind: "cluster", resourceId: null, title: "Cluster" });
  });

  it("tabsList decodes snake_case resource_id from the Rust TabView", async () => {
    const { tabsList } = await import("../../ipc/tabs");
    mocks.invoke.mockResolvedValueOnce([
      {
        id: 1,
        kind: "pipeline",
        resource_id: "7",
        title: "alpha",
        pinned: false,
        position: 0,
      },
    ]);
    const list = await tabsList();
    expect(list).toEqual([
      {
        id: 1,
        kind: "pipeline",
        resource_id: "7",
        title: "alpha",
        pinned: false,
        position: 0,
      },
    ]);
  });
});
