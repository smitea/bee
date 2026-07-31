import { useMemo } from "react";
import ReactECharts from "echarts-for-react";

export interface BarDatum {
  label: string;
  value: number;
}

export interface BarChartProps {
  data: BarDatum[];
  color?: string;
  title?: string;
  height?: number;
}

export function BarChart({
  data,
  color = "#22c55e",
  title,
  height = 200,
}: BarChartProps) {
  const option = useMemo(() => {
    return {
      title: title
        ? { text: title, textStyle: { fontSize: 11, color: "#6b7280" }, top: 2 }
        : undefined,
      grid: { left: 36, right: 8, top: title ? 28 : 6, bottom: 18 },
      tooltip: { trigger: "axis" as const },
      xAxis: {
        type: "category" as const,
        data: data.map((d) => d.label),
        axisLabel: { fontSize: 9 },
      },
      yAxis: { type: "value" as const, axisLabel: { fontSize: 9 } },
      series: [
        {
          type: "bar" as const,
          data: data.map((d) => d.value),
          itemStyle: { color },
        },
      ],
    };
  }, [data, color, title]);

  if (data.length === 0) {
    return (
      <div
        className="flex flex-col items-center justify-center text-[10px] text-gray-400 gap-1"
        style={{ height }}
        data-testid="barchart-empty"
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