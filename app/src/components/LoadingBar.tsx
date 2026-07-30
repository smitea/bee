import type { CSSProperties } from "react";

interface Props {
  label?: string;
}

export function LoadingBar({ label }: Props) {
  const style: CSSProperties = {
    backgroundImage:
      "linear-gradient(90deg, transparent 0%, rgba(59,130,246,0.6) 50%, transparent 100%)",
    backgroundSize: "200% 100%",
    backgroundRepeat: "no-repeat",
    animation: "bee-loading-bar 1.1s linear infinite",
  };
  return (
    <div
      role="status"
      aria-label={label ?? "loading"}
      aria-live="polite"
      className="w-full"
      data-testid="loading-bar"
    >
      <div className="h-[3px] w-full bg-gray-100 dark:bg-neutral-800 rounded">
        <div className="h-full w-1/3 rounded" style={style} />
      </div>
    </div>
  );
}