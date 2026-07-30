import { afterEach, beforeEach, describe, expect, it, vi } from "vitest";
import { fireEvent, render, screen, act } from "@testing-library/react";

const mocks = vi.hoisted(() => ({
  auditRecord: vi.fn(),
  auditList: vi.fn(),
  auditLatest: vi.fn(),
  auditQuery: vi.fn(),
  tabsList: vi.fn(),
  tabOpen: vi.fn(),
  tabClose: vi.fn(),
  tabCloseOthers: vi.fn(),
  tabPin: vi.fn(),
  tabSetActive: vi.fn(),
  workspaceState: vi.fn(),
  setAddr: vi.fn(),
  testConnection: vi.fn(),
  connState: vi.fn(),
  ping: vi.fn(),
  getDefaultAddr: vi.fn(),
}));

vi.mock("../../ipc/audit", () => ({
  auditRecord: mocks.auditRecord,
  auditList: mocks.auditList,
  auditLatest: mocks.auditLatest,
  auditQuery: mocks.auditQuery,
}));

vi.mock("../../ipc/tabs", () => ({
  tabsList: mocks.tabsList,
  tabOpen: mocks.tabOpen,
  tabClose: mocks.tabClose,
  tabCloseOthers: mocks.tabCloseOthers,
  tabPin: mocks.tabPin,
  tabSetActive: mocks.tabSetActive,
  workspaceState: mocks.workspaceState,
}));

vi.mock("../../ipc/connection", () => ({
  connState: mocks.connState,
  setAddr: mocks.setAddr,
  testConnection: mocks.testConnection,
  ping: mocks.ping,
  getDefaultAddr: mocks.getDefaultAddr,
}));

import { useConnection } from "../../state/connectionStore";
import { StatusBar } from "../../components/StatusBar";

function reset() {
  useConnection.setState({
    addr: "127.0.0.1:9999",
    status: { kind: "Connecting" },
    hydrated: false,
  });
  mocks.connState.mockReset();
  mocks.auditList.mockReset();
  mocks.auditLatest.mockReset();
  mocks.tabsList.mockReset();
  mocks.auditList.mockResolvedValue([]);
  mocks.auditLatest.mockResolvedValue(null);
  mocks.tabsList.mockResolvedValue([]);
  mocks.workspaceState.mockResolvedValue({ activeTabId: null });
}

beforeEach(reset);
afterEach(reset);

describe("<StatusBar> reconnect button", () => {
  it("shows the Reconnect button when status is Error", () => {
    useConnection.setState({ status: { kind: "Error", reason: "refused" } });
    render(<StatusBar />);
    expect(screen.getByTestId("statusbar-reconnect")).toBeInTheDocument();
    expect(screen.getByText("Reconnect")).toBeInTheDocument();
  });

  it("shows the Reconnect button when status is Disconnected", () => {
    useConnection.setState({ status: { kind: "Disconnected" } });
    render(<StatusBar />);
    expect(screen.getByTestId("statusbar-reconnect")).toBeInTheDocument();
  });

  it("hides the Reconnect button when status is Connected", () => {
    useConnection.setState({ status: { kind: "Connected" } });
    render(<StatusBar />);
    expect(screen.queryByTestId("statusbar-reconnect")).toBeNull();
  });

  it("hides the Reconnect button when status is Connecting", () => {
    useConnection.setState({ status: { kind: "Connecting" } });
    render(<StatusBar />);
    expect(screen.queryByTestId("statusbar-reconnect")).toBeNull();
  });

  it("clicking the Reconnect button calls refresh() with the current addr", async () => {
    useConnection.setState({
      addr: "10.0.0.1:8888",
      status: { kind: "Error", reason: "refused" },
    });
    mocks.connState.mockResolvedValue({
      addr: "10.0.0.1:8888",
      status: { kind: "Disconnected" },
    });
    render(<StatusBar />);
    const btn = screen.getByTestId("statusbar-reconnect");
    await act(async () => {
      fireEvent.click(btn);
      await Promise.resolve();
    });
    expect(mocks.connState).toHaveBeenCalledWith("10.0.0.1:8888");
  });

  it("shows Reconnecting… text and disables the button while polling", async () => {
    useConnection.setState({ status: { kind: "Disconnected" } });
    mocks.connState.mockResolvedValue({
      addr: "127.0.0.1:9999",
      status: { kind: "Disconnected" },
    });
    render(<StatusBar />);
    const btn = screen.getByTestId("statusbar-reconnect") as HTMLButtonElement;
    await act(async () => {
      fireEvent.click(btn);
      await Promise.resolve();
    });
    expect(btn.textContent).toMatch(/Reconnecting/);
    expect(btn).toBeDisabled();
  });

  it("hides the Reconnect button once refresh transitions to Connected", async () => {
    useConnection.setState({ status: { kind: "Error", reason: "refused" } });
    mocks.connState.mockResolvedValueOnce({
      addr: "127.0.0.1:9999",
      status: { kind: "Connected" },
    });
    render(<StatusBar />);
    expect(screen.getByTestId("statusbar-reconnect")).toBeInTheDocument();
    await act(async () => {
      fireEvent.click(screen.getByTestId("statusbar-reconnect"));
      await Promise.resolve();
    });
    expect(screen.queryByTestId("statusbar-reconnect")).toBeNull();
  });

  it("polls refresh every 4 seconds while still in an error state", async () => {
    vi.useFakeTimers();
    try {
      useConnection.setState({ status: { kind: "Error", reason: "refused" } });
      mocks.connState.mockResolvedValue({
        addr: "127.0.0.1:9999",
        status: { kind: "Error", reason: "refused" },
      });
      render(<StatusBar />);
      await act(async () => {
        fireEvent.click(screen.getByTestId("statusbar-reconnect"));
      });
      const callsAfterClick = mocks.connState.mock.calls.length;
      await act(async () => {
        vi.advanceTimersByTime(4000);
      });
      expect(mocks.connState.mock.calls.length).toBeGreaterThan(callsAfterClick);
      await act(async () => {
        vi.advanceTimersByTime(4000);
      });
      expect(mocks.connState.mock.calls.length).toBeGreaterThan(callsAfterClick + 1);
    } finally {
      vi.useRealTimers();
    }
  });

  it("stops polling once the status transitions to Connected", async () => {
    vi.useFakeTimers();
    try {
      useConnection.setState({ status: { kind: "Disconnected" } });
      let call = 0;
      mocks.connState.mockImplementation(() => {
        call += 1;
        return Promise.resolve({
          addr: "127.0.0.1:9999",
          status: call === 1 ? { kind: "Connected" } : { kind: "Disconnected" },
        });
      });
      render(<StatusBar />);
      await act(async () => {
        fireEvent.click(screen.getByTestId("statusbar-reconnect"));
      });
      expect(mocks.connState.mock.calls.length).toBe(1);
      await act(async () => {
        vi.advanceTimersByTime(8000);
      });
      expect(mocks.connState.mock.calls.length).toBe(1);
    } finally {
      vi.useRealTimers();
    }
  });
});