import { useEffect, useRef } from "react";
import {
  init,
  dispose,
  type Chart,
  type DeepPartial,
  type KLineData,
  type Options,
  type Period,
  type Styles,
} from "klinecharts";

import { useElementWidth } from "../../hooks/useElementWidth";

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
const SUBTLE_GRID = "rgba(120, 120, 120, 0.05)";
const NARROW_PX = 200;
const BAR_SPACE = 4;
const BAR_SPACE_LIMIT_MIN = 2;

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

function buildStyles(
  showLast: boolean,
  narrow: boolean,
  candleType: "candle_solid" | "area",
): DeepPartial<Styles> {
  return {
    grid: {
      show: true,
      horizontal: { show: true, style: "solid", size: 1, color: SUBTLE_GRID, dashedValue: [2, 2] },
      vertical: { show: true, style: "solid", size: 1, color: SUBTLE_GRID, dashedValue: [2, 2] },
    },
    candle: {
      type: candleType,
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
          show: showLast,
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
    xAxis: {
      show: true,
      axisLine: { show: false },
      tickLine: { show: false },
      tickText: {
        show: !narrow,
        color: "#9ca3af",
        size: 9,
        family: "inherit",
        weight: "normal",
        marginStart: 0,
        marginEnd: 0,
      },
    },
    yAxis: {
      show: true,
      axisLine: { show: false },
      tickLine: { show: false },
      tickText: {
        show: !narrow,
        color: "#9ca3af",
        size: 9,
        family: "inherit",
        weight: "normal",
        marginStart: 0,
        marginEnd: 0,
      },
    },
  };
}

function buildInitOptions(): Options {
  return {
    layout: {
      barSpaceLimit: { min: BAR_SPACE_LIMIT_MIN, max: 12 },
    },
  };
}

export function LineChart({
  points,
  mode = "candlestick",
  interval = "1m",
  height = 200,
}: LineChartProps) {
  const containerRef = useRef<HTMLDivElement | null>(null);
  const chartRef = useRef<Chart | null>(null);
  const lastMarkShownRef = useRef<boolean>(false);
  const width = useElementWidth(containerRef);
  const narrow = width > 0 && width < NARROW_PX;

  useEffect(() => {
    if (points.length === 0) return;
    if (!containerRef.current) return;
    const el = containerRef.current;
    const chart = init(el, buildInitOptions());
    if (!chart) return;
    chartRef.current = chart;
    lastMarkShownRef.current = false;

    chart.setSymbol({ ticker: "BTCUSDT", pricePrecision: 2, volumePrecision: 0 });
    chart.setPeriod(PERIOD_MAP[interval]);
    chart.setBarSpace(BAR_SPACE);
    const candleType: "candle_solid" | "area" =
      mode === "line" ? "area" : "candle_solid";
    chart.setStyles(buildStyles(false, narrow, candleType));

    const data = toKLineData(points);
    chart.setDataLoader({
      getBars: ({ callback }) => {
        callback(data);
      },
    });

    const onCrosshairChange = (payload: unknown) => {
      const visible =
        typeof payload === "object" &&
        payload !== null &&
        (payload as { visible?: boolean }).visible === true;
      if (visible === lastMarkShownRef.current) return;
      lastMarkShownRef.current = visible;
      chart.setStyles(buildStyles(visible, narrow, candleType));
    };
    chart.subscribeAction("onCrosshairChange", onCrosshairChange);

    return () => {
      chart.unsubscribeAction("onCrosshairChange", onCrosshairChange);
      if (chartRef.current) {
        dispose(el);
        chartRef.current = null;
      }
    };
  }, [points, mode, interval, narrow]);

  if (points.length === 0) {
    return (
      <div
        className="flex flex-col items-center justify-center text-[10px] text-gray-400 gap-1"
        style={{ height }}
        data-testid="linechart-empty"
      >
        <span className="font-medium">No data yet</span>
        <span className="text-[9px] text-gray-300 dark:text-neutral-500">
          waiting for pipeline rows…
        </span>
      </div>
    );
  }

  return (
    <div
      ref={containerRef}
      data-testid="klinechart"
      data-narrow={narrow ? "true" : "false"}
      style={{ height, width: "100%" }}
    />
  );
}