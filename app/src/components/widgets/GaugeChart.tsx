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
  empty?: boolean;
}

export function GaugeChart({
  value,
  min = 0,
  max = 100,
  unit = "%",
  title,
  height = 180,
  color,
  empty = false,
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

  if (empty) {
    return (
      <div
        className="flex flex-col items-center justify-center text-[10px] text-gray-400 gap-1"
        style={{ height }}
        data-testid="gaugechart-empty"
      >
        <span className="font-medium">No data yet</span>
        <span className="text-[9px] text-gray-300 dark:text-neutral-500">
          waiting for pipeline rows…
        </span>
      </div>
    );
  }

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