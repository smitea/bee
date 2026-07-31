export interface PipelineStatusProps {
  running: number;
  failed: number;
}

export function PipelineStatus({ running, failed }: PipelineStatusProps) {
  return (
    <div
      className="h-full w-full grid grid-cols-2 gap-1"
      data-testid="pipeline-status"
    >
      <div
        className="flex flex-col items-center justify-center h-full text-center"
        data-testid="pipeline-status-running"
      >
        <div
          className="font-semibold tabular-nums leading-none text-accent-green"
          style={{ fontSize: "clamp(1.5rem, 4vw, 2rem)" }}
        >
          {running}
        </div>
        <div className="text-[10px] uppercase tracking-wider text-gray-500 dark:text-neutral-400 mt-1">
          running
        </div>
      </div>
      <div
        className="flex flex-col items-center justify-center h-full text-center"
        data-testid="pipeline-status-failed"
      >
        <div
          className="font-semibold tabular-nums leading-none text-accent-red"
          style={{ fontSize: "clamp(1.5rem, 4vw, 2rem)" }}
        >
          {failed}
        </div>
        <div className="text-[10px] uppercase tracking-wider text-gray-500 dark:text-neutral-400 mt-1">
          failed
        </div>
      </div>
    </div>
  );
}