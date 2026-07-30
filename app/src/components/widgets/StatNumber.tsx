import { useRef } from "react";

import { useElementWidth } from "../../hooks/useElementWidth";

export interface StatNumberProps {
  value: number | string;
  label: string;
  unit?: string;
  trend?: "up" | "down" | "flat";
}

const NARROW_PX = 120;

export function StatNumber({ value, label, unit, trend }: StatNumberProps) {
  const ref = useRef<HTMLDivElement | null>(null);
  const width = useElementWidth(ref);
  const narrow = width > 0 && width < NARROW_PX;

  const trendCls =
    trend === "up"
      ? "text-accent-green"
      : trend === "down"
        ? "text-accent-red"
        : "text-gray-400";
  return (
    <div
      ref={ref}
      className="flex flex-col items-center justify-center h-full px-2 py-1 text-center"
      data-narrow={narrow ? "true" : "false"}
    >
      <div
        className="font-semibold tabular-nums"
        style={{ fontSize: "clamp(0.875rem, 4vw, 1.5rem)" }}
      >
        {value}
        {unit && <span className="text-sm text-gray-500 ml-0.5">{unit}</span>}
      </div>
      <div className="text-[10px] uppercase tracking-wider text-gray-500 dark:text-neutral-400 mt-1">
        {label}
      </div>
      {trend && !narrow && (
        <div className={["text-[10px] font-mono mt-0.5", trendCls].join(" ")}>
          {trend === "up" ? "▲" : trend === "down" ? "▼" : "—"}
        </div>
      )}
    </div>
  );
}