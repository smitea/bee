import { beforeEach, describe, expect, it, vi } from "vitest";
import { fireEvent, render, screen, waitFor } from "@testing-library/react";
import { QueryClient, QueryClientProvider } from "@tanstack/react-query";

const mocks = vi.hoisted(() => ({
  dashboardMetricGet: vi.fn(),
  dashboardMetricSave: vi.fn(),
  listJobs: vi.fn(),
}));

vi.mock("../../ipc/dashboard_metrics", () => ({
  dashboardMetricGet: mocks.dashboardMetricGet,
  dashboardMetricSave: mocks.dashboardMetricSave,
  dashboardMetricList: vi.fn().mockResolvedValue([]),
  dashboardMetricDelete: vi.fn(),
}));

vi.mock("../../ipc/cluster", () => ({
  listJobs: mocks.listJobs,
  clusterStatus: vi.fn(),
  jobInspect: vi.fn(),
}));

import { MetricConfigDialog } from "../../components/MetricConfigDialog";
import { useConnection } from "../../state/connectionStore";

function withClient(node: React.ReactNode) {
  const client = new QueryClient({
    defaultOptions: { queries: { retry: false, gcTime: 0 } },
  });
  return render(<QueryClientProvider client={client}>{node}</QueryClientProvider>);
}

function readWidgetOption(value: string) {
  const select = screen.getByLabelText("widget kind") as HTMLSelectElement;
  fireEvent.change(select, { target: { value } });
}

beforeEach(() => {
  vi.resetModules();
  mocks.dashboardMetricGet.mockReset();
  mocks.dashboardMetricSave.mockReset();
  mocks.listJobs.mockReset();
  mocks.dashboardMetricGet.mockResolvedValue(null);
  mocks.dashboardMetricSave.mockResolvedValue({
    dashboard_id: 1,
    panel_id: "p1",
    pipeline_job_id: null,
    source_field: "price",
    widget_kind: "line_chart",
    chart_config_json: "{}",
    updated_at: 0,
  });
  mocks.listJobs.mockResolvedValue([]);
  useConnection.setState({
    addr: "127.0.0.1:9999",
    status: { kind: "Connected" },
    hydrated: true,
  });
});

describe("<MetricConfigDialog>", () => {
  it("renders all 5 widget kinds in the widget-kind dropdown", () => {
    withClient(<MetricConfigDialog applicationId={1} panelId="p1" onClose={() => {}} />);
    const select = screen.getByLabelText("widget kind") as HTMLSelectElement;
    const values = Array.from(select.querySelectorAll("option")).map((o) => o.value);
    expect(values).toEqual(
      expect.arrayContaining(["line_chart", "kline", "bar_chart", "gauge", "stat"]),
    );
  });

  it("renders mode + interval selects only when widget kind is kline", () => {
    withClient(<MetricConfigDialog applicationId={1} panelId="p1" onClose={() => {}} />);
    expect(screen.queryByLabelText("kline mode")).toBeNull();
    expect(screen.queryByLabelText("kline interval")).toBeNull();

    readWidgetOption("kline");

    const mode = screen.getByLabelText("kline mode") as HTMLSelectElement;
    const interval = screen.getByLabelText("kline interval") as HTMLSelectElement;
    expect(Array.from(mode.querySelectorAll("option")).map((o) => o.value)).toEqual(
      expect.arrayContaining(["candlestick", "line"]),
    );
    expect(Array.from(interval.querySelectorAll("option")).map((o) => o.value)).toEqual(
      expect.arrayContaining(["1m", "5m", "1h", "1d"]),
    );
  });

  it("hides the kline mode + interval selects when the widget kind switches away from kline", () => {
    withClient(<MetricConfigDialog applicationId={1} panelId="p1" onClose={() => {}} />);
    readWidgetOption("kline");
    expect(screen.getByLabelText("kline mode")).toBeInTheDocument();
    readWidgetOption("bar_chart");
    expect(screen.queryByLabelText("kline mode")).toBeNull();
    expect(screen.queryByLabelText("kline interval")).toBeNull();
  });

  it("shows x-axis + y-axis field inputs only when widget kind is bar_chart", () => {
    withClient(<MetricConfigDialog applicationId={1} panelId="p1" onClose={() => {}} />);
    expect(screen.queryByLabelText("x axis field")).toBeNull();
    expect(screen.queryByLabelText("y axis field")).toBeNull();

    readWidgetOption("bar_chart");
    expect(screen.getByLabelText("x axis field")).toBeInTheDocument();
    expect(screen.getByLabelText("x axis label")).toBeInTheDocument();
    expect(screen.getByLabelText("y axis field")).toBeInTheDocument();
    expect(screen.getByLabelText("y axis label")).toBeInTheDocument();

    readWidgetOption("line_chart");
    expect(screen.queryByLabelText("x axis field")).toBeNull();
    expect(screen.queryByLabelText("y axis field")).toBeNull();
  });

  it("shows min / max / value inputs only when widget kind is gauge", () => {
    withClient(<MetricConfigDialog applicationId={1} panelId="p1" onClose={() => {}} />);
    expect(screen.queryByLabelText("gauge min")).toBeNull();
    expect(screen.queryByLabelText("gauge max")).toBeNull();
    expect(screen.queryByLabelText("gauge value")).toBeNull();

    readWidgetOption("gauge");
    expect(screen.getByLabelText("gauge min")).toBeInTheDocument();
    expect(screen.getByLabelText("gauge max")).toBeInTheDocument();
    expect(screen.getByLabelText("gauge value")).toBeInTheDocument();

    readWidgetOption("line_chart");
    expect(screen.queryByLabelText("gauge min")).toBeNull();
    expect(screen.queryByLabelText("gauge max")).toBeNull();
    expect(screen.queryByLabelText("gauge value")).toBeNull();
  });

  it("shows value + delta inputs only when widget kind is stat", () => {
    withClient(<MetricConfigDialog applicationId={1} panelId="p1" onClose={() => {}} />);
    expect(screen.queryByLabelText("stat value")).toBeNull();
    expect(screen.queryByLabelText("stat delta")).toBeNull();
    expect(screen.queryByLabelText("stat trend")).toBeNull();

    readWidgetOption("stat");
    expect(screen.getByLabelText("stat value")).toBeInTheDocument();
    expect(screen.getByLabelText("stat delta")).toBeInTheDocument();
    expect(screen.getByLabelText("stat trend")).toBeInTheDocument();

    readWidgetOption("line_chart");
    expect(screen.queryByLabelText("stat value")).toBeNull();
    expect(screen.queryByLabelText("stat delta")).toBeNull();
    expect(screen.queryByLabelText("stat trend")).toBeNull();
  });

  it("Save forwards bar x/y-axis fields into the chart config", async () => {
    withClient(<MetricConfigDialog applicationId={1} panelId="p1" onClose={() => {}} />);

    readWidgetOption("bar_chart");
    fireEvent.change(screen.getByLabelText("x axis field"), { target: { value: "label" } });
    fireEvent.change(screen.getByLabelText("y axis field"), { target: { value: "value" } });
    fireEvent.click(screen.getByRole("button", { name: /^save$/i }));

    await waitFor(() => expect(mocks.dashboardMetricSave).toHaveBeenCalledTimes(1));
    const cfg = JSON.parse(mocks.dashboardMetricSave.mock.calls[0][5]);
    expect(cfg.xAxisField).toBe("label");
    expect(cfg.yAxisField).toBe("value");
  });

  it("Save forwards gauge min/max/value into the chart config", async () => {
    withClient(<MetricConfigDialog applicationId={1} panelId="p1" onClose={() => {}} />);

    readWidgetOption("gauge");
    fireEvent.change(screen.getByLabelText("gauge min"), { target: { value: "10" } });
    fireEvent.change(screen.getByLabelText("gauge max"), { target: { value: "200" } });
    fireEvent.change(screen.getByLabelText("gauge value"), { target: { value: "75" } });
    fireEvent.click(screen.getByRole("button", { name: /^save$/i }));

    await waitFor(() => expect(mocks.dashboardMetricSave).toHaveBeenCalledTimes(1));
    const cfg = JSON.parse(mocks.dashboardMetricSave.mock.calls[0][5]);
    expect(cfg.min).toBe(10);
    expect(cfg.max).toBe(200);
    expect(cfg.value).toBe(75);
  });

  it("Save forwards stat value + delta + trend into the chart config", async () => {
    withClient(<MetricConfigDialog applicationId={1} panelId="p1" onClose={() => {}} />);

    readWidgetOption("stat");
    fireEvent.change(screen.getByLabelText("stat value"), { target: { value: "42" } });
    fireEvent.change(screen.getByLabelText("stat delta"), { target: { value: "3" } });
    fireEvent.change(screen.getByLabelText("stat trend"), { target: { value: "up" } });
    fireEvent.click(screen.getByRole("button", { name: /^save$/i }));

    await waitFor(() => expect(mocks.dashboardMetricSave).toHaveBeenCalledTimes(1));
    const cfg = JSON.parse(mocks.dashboardMetricSave.mock.calls[0][5]);
    expect(cfg.value).toBe(42);
    expect(cfg.delta).toBe(3);
    expect(cfg.trend).toBe("up");
  });

  it("keeps the shared title / color / unit / source-field inputs visible for every widget kind", () => {
    withClient(<MetricConfigDialog applicationId={1} panelId="p1" onClose={() => {}} />);
    for (const kind of ["line_chart", "kline", "bar_chart", "gauge", "stat"]) {
      readWidgetOption(kind);
      expect(screen.getByLabelText("source field")).toBeInTheDocument();
      expect(screen.getByLabelText("title")).toBeInTheDocument();
      expect(screen.getByLabelText("color")).toBeInTheDocument();
      expect(screen.getByLabelText("unit")).toBeInTheDocument();
    }
  });

  it("hydrates the form from an existing metric record on mount", async () => {
    mocks.dashboardMetricGet.mockResolvedValue({
      dashboard_id: 1,
      panel_id: "p1",
      pipeline_job_id: 7,
      source_field: "close",
      widget_kind: "kline",
      chart_config_json: JSON.stringify({
        title: "BTC",
        color: "#abcdef",
        unit: "$",
        mode: "line",
        interval: "5m",
      }),
      updated_at: 0,
    });

    withClient(<MetricConfigDialog applicationId={1} panelId="p1" onClose={() => {}} />);

    await waitFor(() => {
      expect((screen.getByLabelText("widget kind") as HTMLSelectElement).value).toBe("kline");
    });
    expect((screen.getByLabelText("source field") as HTMLInputElement).value).toBe("close");
    expect((screen.getByLabelText("title") as HTMLInputElement).value).toBe("BTC");
    expect((screen.getByLabelText("kline mode") as HTMLSelectElement).value).toBe("line");
    expect((screen.getByLabelText("kline interval") as HTMLSelectElement).value).toBe("5m");
  });

  it("Save calls dashboardMetricSave with the configured widget kind and chart config", async () => {
    withClient(<MetricConfigDialog applicationId={1} panelId="p1" onClose={() => {}} />);

    fireEvent.change(screen.getByLabelText("source field"), {
      target: { value: "price" },
    });
    fireEvent.change(screen.getByLabelText("title"), {
      target: { value: "BTC" },
    });
    readWidgetOption("line_chart");

    fireEvent.click(screen.getByRole("button", { name: /^save$/i }));

    await waitFor(() => expect(mocks.dashboardMetricSave).toHaveBeenCalledTimes(1));
    const [appId, panelId, jobId, sourceField, widgetKind, chartCfgJson] =
      mocks.dashboardMetricSave.mock.calls[0];
    expect(appId).toBe(1);
    expect(panelId).toBe("p1");
    expect(jobId).toBeNull();
    expect(sourceField).toBe("price");
    expect(widgetKind).toBe("line_chart");
    const cfg = JSON.parse(chartCfgJson);
    expect(cfg.title).toBe("BTC");
    expect(cfg.mode).toBe("candlestick");
    expect(cfg.interval).toBe("1m");
  });

  it("Save forwards the chosen kline mode + interval into the chart config", async () => {
    withClient(<MetricConfigDialog applicationId={1} panelId="p1" onClose={() => {}} />);

    readWidgetOption("kline");
    fireEvent.change(screen.getByLabelText("kline mode"), {
      target: { value: "line" },
    });
    fireEvent.change(screen.getByLabelText("kline interval"), {
      target: { value: "1h" },
    });
    fireEvent.click(screen.getByRole("button", { name: /^save$/i }));

    await waitFor(() => expect(mocks.dashboardMetricSave).toHaveBeenCalledTimes(1));
    const cfg = JSON.parse(mocks.dashboardMetricSave.mock.calls[0][5]);
    expect(cfg.mode).toBe("line");
    expect(cfg.interval).toBe("1h");
  });

  it("Save attaches the selected pipeline job id when one is chosen", async () => {
    mocks.listJobs.mockResolvedValue([
      { job_id: 42, dag_hash: "deadbeef", lifecycle: "Running", mode: "Live", task_count: 1, owner_node: 1 },
    ]);
    withClient(<MetricConfigDialog applicationId={1} panelId="p1" onClose={() => {}} />);

    const select = (await waitFor(() =>
      screen.getByLabelText("pipeline job"),
    )) as HTMLSelectElement;
    await waitFor(() => {
      const opts = Array.from(select.querySelectorAll("option")).map((o) => o.value);
      expect(opts).toContain("42");
    });
    fireEvent.change(select, { target: { value: "42" } });
    expect(select.value).toBe("42");
    fireEvent.click(screen.getByRole("button", { name: /^save$/i }));

    await waitFor(() => expect(mocks.dashboardMetricSave).toHaveBeenCalledTimes(1));
    expect(mocks.dashboardMetricSave.mock.calls[0][2]).toBe(42);
  });

  it("Cancel closes the dialog without invoking save", () => {
    const onClose = vi.fn();
    withClient(<MetricConfigDialog applicationId={1} panelId="p1" onClose={onClose} />);
    fireEvent.click(screen.getByRole("button", { name: /cancel/i }));
    expect(onClose).toHaveBeenCalledTimes(1);
    expect(mocks.dashboardMetricSave).not.toHaveBeenCalled();
  });

  it("renders the dialog header with the panel id", () => {
    withClient(<MetricConfigDialog applicationId={1} panelId="kline" onClose={() => {}} />);
    expect(screen.getByText(/Bind metric · kline/)).toBeInTheDocument();
  });
});
