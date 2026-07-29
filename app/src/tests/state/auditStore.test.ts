import { describe, it, expect, vi, beforeEach } from "vitest";

const auditList = vi.fn();
const auditLatest = vi.fn();

vi.mock("../../ipc/audit", () => ({
  auditList,
  auditLatest,
  auditQuery: vi.fn(),
  auditRecord: vi.fn(),
}));

beforeEach(() => {
  vi.resetModules();
  auditList.mockReset();
  auditLatest.mockReset();
});

function makeEvent(id: number, summary: string, navKind: string | null = null) {
  return {
    id,
    timestamp: 1_700_000_000 + id,
    actor: "tester",
    action: "x",
    result: "Success",
    summary,
    resource_kind: null,
    resource_id: null,
    application_id: null,
    correlation_id: null,
    operation_id: null,
    nav_kind: navKind,
    nav_resource_id: null,
  };
}

describe("useAudit", () => {
  it("refresh populates events from auditList", async () => {
    auditList.mockResolvedValueOnce([makeEvent(2, "b"), makeEvent(1, "a")]);
    const { useAudit } = await import("../../state/auditStore");
    await useAudit.getState().refresh();
    expect(useAudit.getState().events.map((e) => e.id)).toEqual([2, 1]);
    expect(useAudit.getState().loaded).toBe(true);
  });

  it("latest prepends when a new event arrives", async () => {
    auditList.mockResolvedValueOnce([makeEvent(1, "old")]);
    auditLatest.mockResolvedValueOnce(makeEvent(2, "new"));
    const { useAudit } = await import("../../state/auditStore");
    await useAudit.getState().refresh();
    await useAudit.getState().latest();
    expect(useAudit.getState().events.map((e) => e.summary)).toEqual(["new", "old"]);
  });

  it("latest is a no-op when there is no new event", async () => {
    auditList.mockResolvedValueOnce([makeEvent(1, "only")]);
    auditLatest.mockResolvedValueOnce(null);
    const { useAudit } = await import("../../state/auditStore");
    await useAudit.getState().refresh();
    await useAudit.getState().latest();
    expect(useAudit.getState().events.length).toBe(1);
  });
});

describe("navTarget", () => {
  it("returns null when nav_kind is absent", async () => {
    const { navTarget } = await import("../../state/auditStore");
    expect(navTarget(makeEvent(1, "no nav", null))).toBeNull();
  });

  it("returns the navigation target when nav_kind is set", async () => {
    const { navTarget } = await import("../../state/auditStore");
    const ev = { ...makeEvent(2, "with nav", "cluster"), nav_resource_id: "x" };
    expect(navTarget(ev)).toEqual({ kind: "cluster", resourceId: "x" });
  });
});