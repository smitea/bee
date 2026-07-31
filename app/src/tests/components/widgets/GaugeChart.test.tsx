import { beforeEach, describe, expect, it, vi } from "vitest";
import { render } from "@testing-library/react";

interface EChartsMockProps {
  option?: Record<string, unknown>;
  style?: React.CSSProperties;
  opts?: { renderer?: string };
}

vi.mock("echarts-for-react", () => ({
  default: (props: EChartsMockProps) => (
    <div
      data-testid="echarts-mock"
      data-option={JSON.stringify(props.option ?? {})}
      data-renderer={props.opts?.renderer ?? "svg"}
    />
  ),
}));

import {
  GaugeChart,
} from "../../../components/widgets/GaugeChart";
import {
  bandColorFor,
  GAUGE_BAND_GREEN,
  GAUGE_BAND_AMBER,
  GAUGE_BAND_RED,
  GAUGE_BAND_NEUTRAL,
} from "../../../components/widgets/colors";

function readSeriesColor(container: HTMLElement): string | undefined {
  const el = container.querySelector('[data-testid="echarts-mock"]');
  if (!el) return undefined;
  const raw = el.getAttribute("data-option") ?? "{}";
  const option = JSON.parse(raw) as {
    series?: Array<{
      progress?: { itemStyle?: { color?: string } };
    }>;
  };
  return option.series?.[0]?.progress?.itemStyle?.color;
}

describe("bandColorFor (pure)", () => {
  it("maps values in the 0–50% range to the green band", () => {
    expect(bandColorFor(0, 0, 100)).toBe(GAUGE_BAND_GREEN);
    expect(bandColorFor(25, 0, 100)).toBe(GAUGE_BAND_GREEN);
    expect(bandColorFor(49.9, 0, 100)).toBe(GAUGE_BAND_GREEN);
  });

  it("maps values in the 50–80% range to the amber band", () => {
    expect(bandColorFor(50, 0, 100)).toBe(GAUGE_BAND_AMBER);
    expect(bandColorFor(65, 0, 100)).toBe(GAUGE_BAND_AMBER);
    expect(bandColorFor(79.9, 0, 100)).toBe(GAUGE_BAND_AMBER);
  });

  it("maps values in the 80–100% range to the red band", () => {
    expect(bandColorFor(80, 0, 100)).toBe(GAUGE_BAND_RED);
    expect(bandColorFor(90, 0, 100)).toBe(GAUGE_BAND_RED);
    expect(bandColorFor(100, 0, 100)).toBe(GAUGE_BAND_RED);
  });

  it("falls back to the neutral color when min and max are degenerate or values are non-finite", () => {
    expect(bandColorFor(50, 100, 100)).toBe(GAUGE_BAND_NEUTRAL);
    expect(bandColorFor(Number.NaN, 0, 100)).toBe(GAUGE_BAND_NEUTRAL);
    expect(bandColorFor(50, Number.POSITIVE_INFINITY, 100)).toBe(GAUGE_BAND_NEUTRAL);
  });

  it("respects custom min and max when computing the band threshold", () => {
    expect(bandColorFor(2, 0, 5)).toBe(GAUGE_BAND_GREEN);
    expect(bandColorFor(3.5, 0, 5)).toBe(GAUGE_BAND_AMBER);
    expect(bandColorFor(4.5, 0, 5)).toBe(GAUGE_BAND_RED);
  });
});

describe("<GaugeChart>", () => {
  beforeEach(() => {
    vi.clearAllMocks();
  });

  it("mounts with a sample value and renders an echarts canvas container", () => {
    const { container } = render(<GaugeChart value={75} />);
    expect(container.querySelector('[data-testid="echarts-mock"]')).toBeInTheDocument();
    expect(container.querySelector('[data-testid="echarts-mock"]')?.getAttribute("data-renderer")).toBe("canvas");
  });

  it("paints a green band when the value sits in the 0–50% range", () => {
    const { container } = render(<GaugeChart value={25} />);
    expect(readSeriesColor(container)).toBe(GAUGE_BAND_GREEN);
  });

  it("paints an amber band when the value sits in the 50–80% range", () => {
    const { container } = render(<GaugeChart value={65} />);
    expect(readSeriesColor(container)).toBe(GAUGE_BAND_AMBER);
  });

  it("paints a red band when the value sits in the 80–100% range", () => {
    const { container } = render(<GaugeChart value={92} />);
    expect(readSeriesColor(container)).toBe(GAUGE_BAND_RED);
  });

  it("respects an explicit color prop instead of the band color", () => {
    const { container } = render(<GaugeChart value={25} color="#123456" />);
    expect(readSeriesColor(container)).toBe("#123456");
  });

  it("respects custom min and max when picking the band color", () => {
    const { container } = render(<GaugeChart value={1.4} min={0} max={5} unit="/s" title="Tasks/sec" />);
    expect(readSeriesColor(container)).toBe(GAUGE_BAND_GREEN);
  });

  it("falls back to the neutral color when value is non-finite", () => {
    const { container } = render(<GaugeChart value={Number.NaN} />);
    expect(readSeriesColor(container)).toBe(GAUGE_BAND_NEUTRAL);
  });

  it("includes the unit and value in the echarts detail formatter", () => {
    const { container } = render(<GaugeChart value={75} unit="%" />);
    const raw = container.querySelector('[data-testid="echarts-mock"]')?.getAttribute("data-option") ?? "{}";
    const option = JSON.parse(raw) as {
      series?: Array<{
        data?: Array<{ value?: number }>;
        detail?: { formatter?: string };
      }>;
    };
    expect(option.series?.[0]?.data?.[0]?.value).toBe(75);
    expect(option.series?.[0]?.detail?.formatter).toBe("{value}%");
  });

  it("disposes cleanly on unmount", () => {
    const { unmount } = render(<GaugeChart value={75} />);
    expect(() => unmount()).not.toThrow();
  });

  it("reduces the gauge tick count to five and hides axis labels by default", () => {
    const { container } = render(<GaugeChart value={75} />);
    const raw = container.querySelector('[data-testid="echarts-mock"]')?.getAttribute("data-option") ?? "{}";
    const option = JSON.parse(raw) as {
      series?: Array<{
        splitNumber?: number;
        axisLabel?: { show?: boolean };
        progress?: { width?: number };
        axisLine?: { lineStyle?: { width?: number } };
      }>;
    };
    expect(option.series?.[0]?.splitNumber).toBe(4);
    expect(option.series?.[0]?.axisLabel?.show).toBe(false);
    expect(option.series?.[0]?.progress?.width).toBeGreaterThanOrEqual(18);
    expect(option.series?.[0]?.axisLine?.lineStyle?.width).toBeGreaterThanOrEqual(18);
  });

  it("renders the empty-state placeholder when empty=true", () => {
    const { container } = render(<GaugeChart value={50} empty />);
    const empty = container.querySelector('[data-testid="gaugechart-empty"]');
    expect(empty).not.toBeNull();
    expect(empty).toHaveTextContent(/No data yet/);
    expect(empty).toHaveTextContent(/waiting for pipeline rows/i);
    expect(container.querySelector('[data-testid="echarts-mock"]')).toBeNull();
  });

  it("renders the gauge when empty is not set (default false)", () => {
    const { container } = render(<GaugeChart value={50} />);
    expect(container.querySelector('[data-testid="echarts-mock"]')).toBeInTheDocument();
    expect(container.querySelector('[data-testid="gaugechart-empty"]')).toBeNull();
  });
});
