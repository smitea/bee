import { beforeEach, describe, expect, it, vi } from "vitest";
import { fireEvent, render, screen, waitFor } from "@testing-library/react";
import { QueryClient, QueryClientProvider } from "@tanstack/react-query";

vi.mock("echarts-for-react", () => ({
  default: (props: { option?: unknown }) => (
    <div data-testid="echarts-mock" data-option={JSON.stringify(props.option ?? {})} />
  ),
}));

const mocks = vi.hoisted(() => ({
  dashboardGet: vi.fn(),
  dashboardSave: vi.fn(),
  dashboardMetricList: vi.fn().mockResolvedValue([]),
  dashboardMetricGet: vi.fn().mockResolvedValue(null),
  dashboardMetricSave: vi.fn(),
  dashboardMetricDelete: vi.fn(),
}));

vi.mock("../../ipc/dashboards", () => ({
  dashboardGet: mocks.dashboardGet,
  dashboardSave: mocks.dashboardSave,
}));

vi.mock("../../ipc/dashboard_metrics", () => ({
  dashboardMetricList: mocks.dashboardMetricList,
  dashboardMetricGet: mocks.dashboardMetricGet,
  dashboardMetricSave: mocks.dashboardMetricSave,
  dashboardMetricDelete: mocks.dashboardMetricDelete,
}));

import { DashboardEditor } from "../../components/DashboardEditor";

function withClient(node: React.ReactNode) {
  const client = new QueryClient({
    defaultOptions: { queries: { retry: false, gcTime: 0 } },
  });
  return render(<QueryClientProvider client={client}>{node}</QueryClientProvider>);
}

beforeEach(() => {
  vi.resetModules();
  mocks.dashboardGet.mockReset();
  mocks.dashboardSave.mockReset();
  mocks.dashboardMetricList.mockReset();
  mocks.dashboardMetricGet.mockReset();
  mocks.dashboardMetricSave.mockReset();
  mocks.dashboardMetricDelete.mockReset();
  mocks.dashboardGet.mockResolvedValue(null);
  mocks.dashboardSave.mockResolvedValue({
    application_id: 1,
    layout_json: "{}",
    updated_at: 0,
  });
  mocks.dashboardMetricList.mockResolvedValue([]);
  mocks.dashboardMetricGet.mockResolvedValue(null);
  mocks.dashboardMetricSave.mockResolvedValue({
    dashboard_id: 1,
    panel_id: "p1",
    pipeline_job_id: null,
    source_field: "x",
    widget_kind: "line_chart",
    chart_config_json: "{}",
    updated_at: 0,
  });
  mocks.dashboardMetricDelete.mockResolvedValue(undefined);
});

describe("<DashboardEditor>", () => {
  it("renders the default layout when no dashboard is saved yet", async () => {
    withClient(<DashboardEditor applicationId={1} />);
    await screen.findByTestId("dashboard-editor");
    expect(screen.getByTestId("panel-kline")).toBeInTheDocument();
    expect(screen.getByTestId("panel-active_jobs")).toBeInTheDocument();
    expect(screen.getByTestId("panel-tasks_per_sec")).toBeInTheDocument();
    expect(screen.getByTestId("panel-cpu")).toBeInTheDocument();
    expect(screen.getByTestId("panel-pipeline_status")).toBeInTheDocument();
  });

  it("saves the default layout to dashboard_save on first save", async () => {
    withClient(<DashboardEditor applicationId={1} />);
    await screen.findByTestId("dashboard-editor");
    await waitFor(() => expect(mocks.dashboardSave).toHaveBeenCalled());
    const saved = mocks.dashboardSave.mock.calls[0][1];
    const parsed = JSON.parse(saved);
    expect(Array.isArray(parsed.panels)).toBe(true);
    expect(parsed.panels.length).toBe(5);
  });

  it("adds a panel and saves the updated layout", async () => {
    withClient(<DashboardEditor applicationId={1} />);
    await screen.findByTestId("dashboard-editor");
    const select = await screen.findByTestId("add-panel-select");
    fireEvent.change(select, { target: { value: "audit_feed" } });
    await waitFor(() => {
      const calls = mocks.dashboardSave.mock.calls;
      const last = JSON.parse(calls[calls.length - 1][1]);
      expect(last.panels.some((p: { kind: string }) => p.kind === "audit_feed")).toBe(true);
    });
  });

  it("removes a panel via the panel header menu", async () => {
    withClient(<DashboardEditor applicationId={1} />);
    await screen.findByTestId("panel-kline");
    fireEvent.click(screen.getByTestId("panel-menu-kline"));
    fireEvent.click(screen.getByTestId("panel-remove-kline"));
    await waitFor(() => {
      const calls = mocks.dashboardSave.mock.calls;
      const last = JSON.parse(calls[calls.length - 1][1]);
      expect(last.panels.some((p: { id: string }) => p.id === "kline")).toBe(false);
    });
  });

  it("repositions a panel via mouse drag and saves the new layout", async () => {
    withClient(<DashboardEditor applicationId={1} />);
    await screen.findByTestId("panel-kline");
    const panel = screen.getByTestId("panel-kline");
    const header = panel.querySelector("header") as HTMLElement;
    header.getBoundingClientRect = () =>
      ({ left: 0, top: 0, right: 100, bottom: 30, width: 100, height: 30, x: 0, y: 0, toJSON: () => "" }) as DOMRect;
    fireEvent.mouseDown(header, { clientX: 0, clientY: 0 });
    fireEvent.mouseMove(document, { clientX: 240, clientY: 240 });
    fireEvent.mouseUp(document);
    await waitFor(() => {
      const calls = mocks.dashboardSave.mock.calls;
      const last = JSON.parse(calls[calls.length - 1][1]);
      const kline = last.panels.find((p: { id: string }) => p.id === "kline");
      expect(kline.x).toBeGreaterThan(0);
    });
  });

  it("round-trips a saved layout through dashboard_get + dashboard_save", async () => {
    mocks.dashboardGet.mockResolvedValue({
      application_id: 1,
      layout_json: JSON.stringify({
        panels: [
          { id: "p1", kind: "kline", x: 1, y: 1, w: 4, h: 3, title: "K-line" },
          { id: "p2", kind: "cpu", x: 5, y: 1, w: 3, h: 2, title: "CPU" },
        ],
      }),
      updated_at: 0,
    });
    withClient(<DashboardEditor applicationId={1} />);
    expect(await screen.findByTestId("panel-p1")).toBeInTheDocument();
    expect(screen.getByTestId("panel-p2")).toBeInTheDocument();
    fireEvent.click(screen.getByTestId("panel-menu-p1"));
    fireEvent.click(screen.getByTestId("panel-remove-p1"));
    await waitFor(() => {
      const calls = mocks.dashboardSave.mock.calls;
      const last = JSON.parse(calls[calls.length - 1][1]);
      expect(last.panels.map((p: { id: string }) => p.id)).toEqual(["p2"]);
    });
  });
});
