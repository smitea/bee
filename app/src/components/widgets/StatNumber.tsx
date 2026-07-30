export interface StatNumberProps {
  value: number | string;
  label: string;
  unit?: string;
  trend?: "up" | "down" | "flat";
}

export function StatNumber({ value, label, unit, trend }: StatNumberProps) {
  const trendCls =
    trend === "up"
      ? "text-accent-green"
      : trend === "down"
        ? "text-accent-red"
        : "text-gray-400";
  return (
    <div className="flex flex-col items-center justify-center h-full px-2 py-1 text-center">
      <div className="text-2xl font-semibold tabular-nums">
        {value}
        {unit && <span className="text-sm text-gray-500 ml-0.5">{unit}</span>}
      </div>
      <div className="text-[10px] uppercase tracking-wider text-gray-500 dark:text-neutral-400 mt-1">
        {label}
      </div>
      {trend && (
        <div className={["text-[10px] font-mono mt-0.5", trendCls].join(" ")}>
          {trend === "up" ? "▲" : trend === "down" ? "▼" : "—"}
        </div>
      )}
    </div>
  );
}