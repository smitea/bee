import { describe, expect, it, vi } from "vitest";
import { afterEach } from "vitest";
import { fireEvent, render, screen } from "@testing-library/react";

const mocks = vi.hoisted(() => ({
  auditRecord: vi.fn(),
  auditList: vi.fn(),
  auditLatest: vi.fn(),
  auditQuery: vi.fn(),
}));

vi.mock("../../ipc/audit", () => ({
  auditRecord: mocks.auditRecord,
  auditList: mocks.auditList,
  auditLatest: mocks.auditLatest,
  auditQuery: mocks.auditQuery,
}));

import { ErrorBoundary } from "../../components/ErrorBoundary";

function Bomb({ shouldThrow }: { shouldThrow: boolean }) {
  if (shouldThrow) throw new Error("boom");
  return <div data-testid="child-ok">child ok</div>;
}

afterEach(() => {
  mocks.auditRecord.mockReset();
});

describe("<ErrorBoundary>", () => {
  it("renders children when no error is thrown", () => {
    render(
      <ErrorBoundary>
        <Bomb shouldThrow={false} />
      </ErrorBoundary>,
    );
    expect(screen.getByTestId("child-ok")).toBeInTheDocument();
    expect(screen.queryByTestId("error-boundary-fallback")).toBeNull();
  });

  it("renders a fallback when a child throws", () => {
    render(
      <ErrorBoundary>
        <Bomb shouldThrow />
      </ErrorBoundary>,
    );
    expect(screen.getByTestId("error-boundary-fallback")).toBeInTheDocument();
    expect(screen.getByText("boom")).toBeInTheDocument();
    expect(screen.queryByTestId("child-ok")).toBeNull();
  });

  it("logs the caught error to console and audit", () => {
    const consoleSpy = vi.spyOn(console, "error").mockImplementation(() => undefined);
    mocks.auditRecord.mockResolvedValueOnce(1);
    render(
      <ErrorBoundary label="dashboard">
        <Bomb shouldThrow />
      </ErrorBoundary>,
    );
    expect(consoleSpy).toHaveBeenCalled();
    expect(mocks.auditRecord).toHaveBeenCalledWith(
      expect.objectContaining({
        actor: "bee-client",
        action: "ui.error",
        result: "Failure",
        summary: expect.stringContaining("dashboard: boom"),
      }),
    );
    consoleSpy.mockRestore();
  });

  it("clicking Reset recovers and re-renders the children", () => {
    let shouldThrow = true;
    function Toggle() {
      if (shouldThrow) throw new Error("first");
      return <div data-testid="child-ok">child ok</div>;
    }
    const { rerender } = render(
      <ErrorBoundary>
        <Toggle />
      </ErrorBoundary>,
    );
    expect(screen.getByTestId("error-boundary-fallback")).toBeInTheDocument();
    shouldThrow = false;
    fireEvent.click(screen.getByTestId("error-boundary-reset"));
    rerender(
      <ErrorBoundary>
        <Toggle />
      </ErrorBoundary>,
    );
    expect(screen.getByTestId("child-ok")).toBeInTheDocument();
    expect(screen.queryByTestId("error-boundary-fallback")).toBeNull();
  });

  it("uses a custom fallback when provided", () => {
    render(
      <ErrorBoundary
        fallback={(err, reset) => (
          <div>
            <span data-testid="custom-fallback-msg">{err.message}</span>
            <button data-testid="custom-reset" onClick={reset}>Custom Reset</button>
          </div>
        )}
      >
        <Bomb shouldThrow />
      </ErrorBoundary>,
    );
    expect(screen.getByTestId("custom-fallback-msg")).toHaveTextContent("boom");
    expect(screen.queryByTestId("error-boundary-fallback")).toBeNull();
  });
});