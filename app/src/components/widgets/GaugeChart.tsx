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

export function GaugeChart({
  value,
  min = 0,
  max = 100,
  unit = "%",
  title,
  height = 180,
  color = "#3b82f6",
}: GaugeChartProps) {
  const option = useMemo(() => {
    return {
      title: title
        ? { text: title, textStyle: { fontSize: 11, color: "#6b7280" }, top: 2 }
        : undefined,
      series: [
        {
          type: "gauge" as const,
          min,
          max,
          progress: { show: true, width: 8, itemStyle: { color } },
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