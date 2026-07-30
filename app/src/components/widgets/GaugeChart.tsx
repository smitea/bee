import { useMemo } from "react";
import ReactECharts from "echarts-for-react";

import { bandColorFor } from "./colors";

export interface GaugeChartProps {
  value: number;
  min?: number;
  max?: number;
  unit?: string;
  title?: string;
  height?: number;
  color?: string;
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
          splitNumber: 4,
          progress: { show: true, width: 18, itemStyle: { color: effectiveColor } },
          axisLine: { lineStyle: { width: 18 } },
          pointer: {
            show: true,
            length: "55%",
            width: 4,
            itemStyle: { color: "#374151" },
          },
          axisTick: { show: false },
          splitLine: { length: 8, lineStyle: { width: 1, color: "#9ca3af" } },
          axisLabel: { show: false },
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