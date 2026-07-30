import { beforeEach, describe, expect, it, vi } from "vitest";
import { render, screen } from "@testing-library/react";

const chartMock = {
  setSymbol: vi.fn(),
  setPeriod: vi.fn(),
  setDataLoader: vi.fn(),
  setStyles: vi.fn(),
  setFormatter: vi.fn(),
  subscribeAction: vi.fn(),
  resetData: vi.fn(),
  getDataList: vi.fn(() => []),
};

const mocks = vi.hoisted(() => ({
  init: vi.fn(() => chartMock),
  dispose: vi.fn(),
}));

vi.mock("klinecharts", () => ({
  init: mocks.init,
  dispose: mocks.dispose,
}));

import { LineChart } from "../../../components/widgets/LineChart";

const sampleOhlc = [
  { ts: 1, open: 100, high: 110, low: 95, close: 105, volume: 1000 },
  { ts: 2, open: 105, high: 115, low: 100, close: 110, volume: 1100 },
  { ts: 3, open: 110, high: 112, low: 99, close: 101, volume: 800 },
];

const sampleSeries = [
  { ts: 1, value: 100 },
  { ts: 2, value: 105 },
  { ts: 3, value: 101 },
];

describe("<LineChart>", () => {
  beforeEach(() => {
    vi.clearAllMocks();
    mocks.init.mockReturnValue(chartMock);
  });

  it("renders a klinecharts canvas container when sample data is provided", () => {
    const { container } = render(<LineChart points={sampleOhlc} mode="candlestick" />);
    expect(container.querySelector("[data-testid=klinechart]")).toBeInTheDocument();
    expect(mocks.init).toHaveBeenCalledTimes(1);
  });

  it("falls back to an empty state when no data is provided", () => {
    render(<LineChart points={[]} mode="candlestick" />);
    expect(screen.getByText(/no data/i)).toBeInTheDocument();
    expect(mocks.init).not.toHaveBeenCalled();
  });

  it("configures the candle chart type as candlestick when mode='candlestick'", () => {
    render(<LineChart points={sampleOhlc} mode="candlestick" />);
    const styles = chartMock.setStyles.mock.calls[0][0];
    expect(styles.candle.type).toBe("candle_solid");
  });

  it("configures the candle chart type as area when mode='line'", () => {
    render(<LineChart points={sampleSeries} mode="line" />);
    const styles = chartMock.setStyles.mock.calls[0][0];
    expect(styles.candle.type).toBe("area");
  });

  it("uses the design-token green up color and red down color", () => {
    render(<LineChart points={sampleOhlc} mode="candlestick" />);
    const styles = chartMock.setStyles.mock.calls[0][0];
    expect(styles.candle.bar.upColor).toBe("#22c55e");
    expect(styles.candle.bar.downColor).toBe("#ef4444");
  });

  it("disposes the chart on unmount", () => {
    const { unmount } = render(<LineChart points={sampleOhlc} mode="candlestick" />);
    unmount();
    expect(mocks.dispose).toHaveBeenCalled();
  });

  it("paints OHLC entries via setDataLoader when in candlestick mode", () => {
    render(<LineChart points={sampleOhlc} mode="candlestick" />);
    expect(chartMock.setDataLoader).toHaveBeenCalledTimes(1);
    const loader = chartMock.setDataLoader.mock.calls[0][0];
    const bars: Array<Record<string, number>> = [];
    loader.getBars({ callback: (d: Array<Record<string, number>>) => bars.push(...d) });
    expect(bars.length).toBe(sampleOhlc.length);
    expect(bars[0]).toMatchObject({
      timestamp: 1,
      open: 100,
      high: 110,
      low: 95,
      close: 105,
      volume: 1000,
    });
  });

  it("paints single-value entries via setDataLoader when in line mode", () => {
    render(<LineChart points={sampleSeries} mode="line" />);
    expect(chartMock.setDataLoader).toHaveBeenCalledTimes(1);
    const loader = chartMock.setDataLoader.mock.calls[0][0];
    const bars: Array<Record<string, number>> = [];
    loader.getBars({ callback: (d: Array<Record<string, number>>) => bars.push(...d) });
    expect(bars.length).toBe(sampleSeries.length);
    expect(bars[0].close).toBe(100);
    expect(bars[1].close).toBe(105);
    expect(bars[2].close).toBe(101);
  });

  it("re-renders when the data points change", () => {
    const { rerender } = render(<LineChart points={sampleOhlc} mode="candlestick" />);
    expect(mocks.init).toHaveBeenCalledTimes(1);
    expect(mocks.dispose).not.toHaveBeenCalled();

    const updatedOhlc = [
      ...sampleOhlc,
      { ts: 4, open: 101, high: 120, low: 100, close: 115, volume: 1200 },
    ];
    rerender(<LineChart points={updatedOhlc} mode="candlestick" />);

    expect(mocks.dispose).toHaveBeenCalledTimes(1);
    expect(mocks.init).toHaveBeenCalledTimes(2);

    const lastLoader = chartMock.setDataLoader.mock.calls[chartMock.setDataLoader.mock.calls.length - 1]?.[0];
    expect(lastLoader).toBeDefined();
    const bars: Array<Record<string, number>> = [];
    lastLoader.getBars({
      callback: (d: Array<Record<string, number>>) => bars.push(...d),
    });
    expect(bars.length).toBe(updatedOhlc.length);
    expect(bars[bars.length - 1]).toMatchObject({ timestamp: 4, close: 115 });
  });

  it("switches the configured period when the interval prop changes", () => {
    const { rerender } = render(<LineChart points={sampleOhlc} mode="candlestick" interval="1m" />);
    expect(chartMock.setPeriod).toHaveBeenLastCalledWith({ type: "minute", span: 1 });

    rerender(<LineChart points={sampleOhlc} mode="candlestick" interval="5m" />);
    expect(chartMock.setPeriod).toHaveBeenLastCalledWith({ type: "minute", span: 5 });

    rerender(<LineChart points={sampleOhlc} mode="candlestick" interval="1h" />);
    expect(chartMock.setPeriod).toHaveBeenLastCalledWith({ type: "hour", span: 1 });

    rerender(<LineChart points={sampleOhlc} mode="candlestick" interval="1d" />);
    expect(chartMock.setPeriod).toHaveBeenLastCalledWith({ type: "day", span: 1 });
  });
});
