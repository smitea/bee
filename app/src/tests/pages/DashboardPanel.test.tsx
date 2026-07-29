import { describe, it, expect, vi, beforeEach } from "vitest";
import { render, screen, fireEvent, waitFor } from "@testing-library/react";

const mocks = vi.hoisted(() => ({
  pipelineLatestResult: vi.fn(),
}));

vi.mock("../../ipc/pipelines", () => ({
  pipelineLatestResult: mocks.pipelineLatestResult,
}));

import { DashboardPanel } from "../../pages/DashboardPanel";

beforeEach(() => {
  vi.resetModules();
  mocks.pipelineLatestResult.mockReset();
});

describe("<DashboardPanel>", () => {
  it("renders in loading state initially", () => {
    mocks.pipelineLatestResult.mockImplementation(
      () => new Promise(() => {}),
    );
    render(
      <DashboardPanel addr="127.0.0.1:9999" jobId={1} label="Latest price" />,
    );
    expect(screen.getByText(/loading/i)).toBeInTheDocument();
    expect(screen.getByText("Latest price")).toBeInTheDocument();
  });

  it("renders the numeric value once the fetch resolves", async () => {
    mocks.pipelineLatestResult.mockResolvedValue({
      numeric: 42.5,
      label: "Latest price",
    });
    render(
      <DashboardPanel addr="127.0.0.1:9999" jobId={1} label="Latest price" />,
    );
    const numeric = await screen.findByTestId("dashboard-numeric");
    expect(numeric.textContent).toBe("42.50");
    expect(screen.queryByText(/loading/i)).toBeNull();
  });

  it("renders an error state when the fetch fails", async () => {
    mocks.pipelineLatestResult.mockRejectedValue(new Error("boom"));
    render(
      <DashboardPanel addr="127.0.0.1:9999" jobId={1} label="Latest price" />,
    );
    expect(await screen.findByText(/failed/i)).toBeInTheDocument();
  });

  it("pause stops polling and shows paused state", async () => {
    mocks.pipelineLatestResult.mockResolvedValue({ numeric: 7, label: "x" });
    render(
      <DashboardPanel addr="127.0.0.1:9999" jobId={1} label="Latest price" />,
    );
    await screen.findByTestId("dashboard-numeric");
    fireEvent.click(screen.getByRole("button", { name: /pause/i }));
    expect(await screen.findByTestId("paused-state")).toBeInTheDocument();
  });

  it("resume re-enables polling after pause", async () => {
    mocks.pipelineLatestResult.mockResolvedValue({ numeric: 9, label: "x" });
    render(
      <DashboardPanel addr="127.0.0.1:9999" jobId={1} label="Latest price" />,
    );
    await screen.findByTestId("dashboard-numeric");
    fireEvent.click(screen.getByRole("button", { name: /pause/i }));
    await screen.findByTestId("paused-state");
    fireEvent.click(screen.getByRole("button", { name: /resume/i }));
    await waitFor(() =>
      expect(screen.queryByTestId("paused-state")).toBeNull(),
    );
  });

  it("renders gracefully when server returns null", async () => {
    mocks.pipelineLatestResult.mockResolvedValue(null);
    render(
      <DashboardPanel addr="127.0.0.1:9999" jobId={1} label="Latest price" />,
    );
    expect(await screen.findByText(/no data/i)).toBeInTheDocument();
  });
});
