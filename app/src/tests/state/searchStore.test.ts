import { afterEach, beforeEach, describe, expect, it, vi } from "vitest";

const mocks = vi.hoisted(() => ({
  searchLocal: vi.fn(),
  searchServer: vi.fn(),
}));

vi.mock("../../ipc/search", () => ({
  searchLocal: mocks.searchLocal,
  searchServer: mocks.searchServer,
}));

import { useSearch } from "../../state/searchStore";

function reset() {
  useSearch.setState({ query: "", loading: false, results: [] });
  mocks.searchLocal.mockReset();
  mocks.searchServer.mockReset();
  useSearch.setState({ query: "", loading: false, results: [] });
}

describe("useSearch", () => {
  beforeEach(reset);
  afterEach(reset);

  it("initial state is empty and idle", () => {
    expect(useSearch.getState().query).toBe("");
    expect(useSearch.getState().loading).toBe(false);
    expect(useSearch.getState().results).toEqual([]);
  });

  it("setQuery updates only the query", () => {
    useSearch.getState().setQuery("alpha");
    expect(useSearch.getState().query).toBe("alpha");
    expect(useSearch.getState().loading).toBe(false);
  });

  it("debounces and triggers search after the wait", async () => {
    vi.useFakeTimers();
    try {
      const localHit = { kind: "Pipeline", id: "1", title: "alpha", path: ["Pipelines"] };
      const serverHit = {
        kind: "ClusterNode",
        id: "127.0.0.1:9999",
        title: "127.0.0.1:9999",
        path: ["Cluster"],
      };
      mocks.searchLocal.mockResolvedValueOnce([localHit]);
      mocks.searchServer.mockResolvedValueOnce([serverHit]);

      useSearch.getState().setQuery("alp");
      await vi.advanceTimersByTimeAsync(250);

      await vi.waitFor(() =>
        expect(useSearch.getState().results.length).toBeGreaterThan(0),
      );
      expect(useSearch.getState().loading).toBe(false);
      const kinds = useSearch.getState().results.map((r) => r.kind).sort();
      expect(kinds).toEqual(["ClusterNode", "Pipeline"]);
    } finally {
      vi.useRealTimers();
    }
  });

  it("cancellation: stale local response does not overwrite fresh results", async () => {
    let resolveLocal: (hits: { kind: string; id: string; title: string; path: string[] }[]) => void = () => {};
    const stale = [{ kind: "Pipeline", id: "stale", title: "old", path: ["P"] }];
    mocks.searchLocal.mockImplementationOnce(
      () =>
        new Promise<typeof stale>((r) => {
          resolveLocal = r;
        }),
    );
    const fresh = [{ kind: "Pipeline", id: "2", title: "new", path: ["P"] }];
    mocks.searchLocal.mockResolvedValueOnce(fresh);
    mocks.searchServer.mockResolvedValue([]);

    const stalePromise = useSearch.getState().runSearchNow("old");
    await useSearch.getState().runSearchNow("new");
    expect(useSearch.getState().results[0]?.id).toBe("2");
    resolveLocal(stale);
    await stalePromise.catch(() => {});
    expect(useSearch.getState().results[0]?.id).toBe("2");
  });

  it("merge order: higher explicit score first", () => {
    const merged = useSearch.getState().merge([
      { kind: "Pipeline", id: "1", title: "alpha", path: ["P"], _score: 0.2 } as never,
      { kind: "ClusterNode", id: "x", title: "x", path: ["Cluster"], _score: 0.9 } as never,
      { kind: "Pipeline", id: "2", title: "alpha-pipeline", path: ["P"], _score: 0.5 } as never,
    ]);
    expect(merged.map((r) => r.id)).toEqual(["x", "2", "1"]);
  });

  it("empty query clears results", async () => {
    useSearch.getState().setQuery("alpha");
    await useSearch.getState().runSearchNow("alpha");
    await useSearch.getState().runSearchNow("");
    expect(useSearch.getState().results).toEqual([]);
  });

  it("IPC failures are swallowed and results reset", async () => {
    mocks.searchLocal.mockRejectedValueOnce(new Error("boom"));
    mocks.searchServer.mockRejectedValueOnce(new Error("also boom"));
    await useSearch.getState().runSearchNow("abc");
    expect(useSearch.getState().results).toEqual([]);
    expect(useSearch.getState().loading).toBe(false);
  });
});
