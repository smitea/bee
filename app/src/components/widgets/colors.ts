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