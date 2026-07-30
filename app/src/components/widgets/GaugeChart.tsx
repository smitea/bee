import { useMemo } from "react";
import ReactECharts from "echarts-for-react";

export interface GaugeChartProps {
  value: number;
  min?: number;
  max?: number;
  unit?: string;
  title?: string;
  height?: number;
  color?: string;
}

export const GAUGE_BAND_GREEN = "#22c55e";
export const GAUGE_BAND_AMBER = "#f59e0b";
export const GAUGE_BAND_RED = "#ef4444";
export const GAUGE_BAND_NEUTRAL = "#3b82f6";

export function bandColorFor(value: number, min: number, max: number): string {
  if (!Number.isFinite(value) || !Number.isFinite(min) || !Number.isFinite(max)) {
    return GAUGE_BAND_NEUTRAL;
  }
  const range = max - min;
  if (range <= 0) return GAUGE_BAND_NEUTRAL;
  const pct = ((value - min) / range) * 100;
  if (pct < 50) return GAUGE_BAND_GREEN;
  if (pct < 80) return GAUGE_BAND_AMBER;
  return GAUGE_BAND_RED;
}

export function GaugeChart({
  value,
  min = 0,
  max = 100,
  unit = "%",
  title,
  height = 180,
  color,
}: GaugeChartProps) {
  const option = useMemo(() => {
    const effectiveColor = color ?? bandColorFor(value, min, max);
    return {
      title: title
        ? { text: title, textStyle: { fontSize: 11, color: "#6b7280" }, top: 2 }
        : undefined,
      series: [
        {
          type: "gauge" as const,
          min,
          max,
          progress: { show: true, width: 8, itemStyle: { color: effectiveColor } },
          axisLine: { lineStyle: { width: 8 } },
          pointer: { show: false },
          axisTick: { show: false },
          splitLine: { show: false },
          axisLabel: { fontSize: 9, distance: 12 },
          anchor: { show: false },
          title: { show: false },
          detail: {
            valueAnimation: true,
            fontSize: 16,
            color: "#374151",
            offsetCenter: [0, "0%"],
            formatter: `{value}${unit}`,
          },
          data: [{ value }],
        },
      ],
    };
  }, [value, min, max, unit, title, color]);

  return (
    <ReactECharts
      option={option}
      notMerge
      lazyUpdate
      style={{ height, width: "100%" }}
      opts={{ renderer: "canvas" }}
    />
  );
}