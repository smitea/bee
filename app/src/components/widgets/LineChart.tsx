import { useMemo } from "react";
import ReactECharts from "echarts-for-react";

export interface SeriesPoint {
  ts: number;
  value: number;
}

export interface LineChartProps {
  points: SeriesPoint[];
  color?: string;
  title?: string;
  height?: number;
}

export function LineChart({
  points,
  color = "#3b82f6",
  title,
  height = 200,
}: LineChartProps) {
  const option = useMemo(() => {
    return {
      title: title
        ? { text: title, textStyle: { fontSize: 11, color: "#6b7280" }, top: 2 }
        : undefined,
      grid: { left: 30, right: 8, top: title ? 28 : 6, bottom: 18 },
      tooltip: { trigger: "axis" as const },
      xAxis: {
        type: "time" as const,
        axisLabel: { fontSize: 9 },
      },
      yAxis: { type: "value" as const, axisLabel: { fontSize: 9 } },
      series: [
        {
          type: "line" as const,
          showSymbol: false,
          smooth: true,
          data: points.map((p) => [p.ts, p.value]),
          lineStyle: { color, width: 1.5 },
          areaStyle: { color, opacity: 0.1 },
        },
      ],
    };
  }, [points, color, title]);

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
    <ReactECharts
      option={option}
      notMerge
      lazyUpdate
      style={{ height, width: "100%" }}
      opts={{ renderer: "canvas" }}
    />
  );
}