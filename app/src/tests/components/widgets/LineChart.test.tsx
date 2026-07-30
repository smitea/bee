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
});
