import { afterEach, beforeEach, describe, expect, it, vi } from "vitest";

const connStateMock = vi.fn();
vi.mock("../ipc", async () => {
  const actual = await vi.importActual<typeof import("../ipc")>("../ipc");
  return {
    ...actual,
    connState: (addr: string) => connStateMock(addr),
  };
});

import { useConnection } from "./connectionStore";

function reset() {
  useConnection.setState({
    addr: "127.0.0.1:9999",
    status: { kind: "Connecting" },
    hydrated: false,
  });
  connStateMock.mockReset();
}

describe("connectionStore", () => {
  beforeEach(reset);
  afterEach(reset);

  it("uses the env-default addr initially", () => {
    expect(useConnection.getState().addr).toBe("127.0.0.1:9999");
    expect(useConnection.getState().status.kind).toBe("Connecting");
  });

  it("refresh updates addr and status from a successful probe", async () => {
    const view: import("../ipc").StateView = {
      addr: "127.0.0.1:9999",
      status: { kind: "Connected" },
    };
    connStateMock.mockResolvedValueOnce(view);
    await useConnection.getState().refresh("127.0.0.1:9999");
    expect(useConnection.getState().status).toEqual({ kind: "Connected" });
    expect(connStateMock).toHaveBeenCalledWith("127.0.0.1:9999");
  });

  it("refresh marks Disconnected on probe failure", async () => {
    connStateMock.mockRejectedValueOnce(new Error("boom"));
    await useConnection.getState().refresh("127.0.0.1:9999");
    expect(useConnection.getState().status).toEqual({ kind: "Disconnected" });
  });

  it("setAddr updates only the cached addr", () => {
    useConnection.getState().setAddr("10.0.0.5:8888");
    expect(useConnection.getState().addr).toBe("10.0.0.5:8888");
  });
});
