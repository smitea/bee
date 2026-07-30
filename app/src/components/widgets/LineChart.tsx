import { useEffect, useRef } from "react";
import {
  init,
  dispose,
  type Chart,
  type DeepPartial,
  type KLineData,
  type Period,
  type Styles,
} from "klinecharts";

export type KLineMode = "candlestick" | "line";
export type KLineInterval = "1m" | "5m" | "1h" | "1d";

export interface OhlcPoint {
  ts: number;
  open: number;
  high: number;
  low: number;
  close: number;
  volume?: number;
}

export interface SeriesPoint {
  ts: number;
  value: number;
}

export type KLineInput = OhlcPoint | SeriesPoint;

export interface LineChartProps {
  points: KLineInput[];
  mode?: KLineMode;
  interval?: KLineInterval;
  height?: number;
}

const UP_COLOR = "#22c55e";
const DOWN_COLOR = "#ef4444";

const PERIOD_MAP: Record<KLineInterval, Period> = {
  "1m": { type: "minute", span: 1 },
  "5m": { type: "minute", span: 5 },
  "1h": { type: "hour", span: 1 },
  "1d": { type: "day", span: 1 },
};

function isOhlc(p: KLineInput): p is OhlcPoint {
  return typeof (p as OhlcPoint).open === "number";
}

function toKLineData(points: KLineInput[]): KLineData[] {
  return points.map((p) => {
    if (isOhlc(p)) {
      return {
        timestamp: p.ts,
        open: p.open,
        high: p.high,
        low: p.low,
        close: p.close,
        volume: p.volume ?? 0,
      };
    }
    const v = (p as SeriesPoint).value;
    return {
      timestamp: p.ts,
      open: v,
      high: v,
      low: v,
      close: v,
      volume: 0,
    };
  });
}

export function LineChart({
  points,
  mode = "candlestick",
  interval = "1m",
  height = 200,
}: LineChartProps) {
  const containerRef = useRef<HTMLDivElement | null>(null);
  const chartRef = useRef<Chart | null>(null);

  useEffect(() => {
    if (points.length === 0) return;
    if (!containerRef.current) return;
    const el = containerRef.current;
    const chart = init(el);
    if (!chart) return;
    chartRef.current = chart;

    chart.setSymbol({ ticker: "BTCUSDT", pricePrecision: 2, volumePrecision: 0 });
    chart.setPeriod(PERIOD_MAP[interval]);

    const styles: DeepPartial<Styles> = {
      candle: {
        type: mode === "line" ? "area" : "candle_solid",
        bar: {
          upColor: UP_COLOR,
          downColor: DOWN_COLOR,
          upBorderColor: UP_COLOR,
          downBorderColor: DOWN_COLOR,
          upWickColor: UP_COLOR,
          downWickColor: DOWN_COLOR,
          noChangeColor: "#888888",
        },
        area: {
          lineSize: 2,
          lineColor: UP_COLOR,
          smooth: true,
          value: "close",
        },
        priceMark: {
          last: {
            upColor: UP_COLOR,
            downColor: DOWN_COLOR,
            noChangeColor: "#888888",
          },
        },
      },
      indicator: {
        ohlc: {
          upColor: UP_COLOR,
          downColor: DOWN_COLOR,
          noChangeColor: "#888888",
        },
      },
    };
    chart.setStyles(styles);

    const data = toKLineData(points);
    chart.setDataLoader({
      getBars: ({ callback }) => {
        callback(data);
      },
    });

    return () => {
      if (chartRef.current) {
        dispose(el);
        chartRef.current = null;
      }
    };
  }, [points, mode, interval]);

  if (points.length === 0) {
    return (
      <div
        className="flex items-center justify-center text-[10px] text-gray-400"
        style={{ height }}
      >
        no data
      </div>
    );
  }

  return (
    <div
      ref={containerRef}
      data-testid="klinechart"
      style={{ height, width: "100%" }}
    />
  );
}
