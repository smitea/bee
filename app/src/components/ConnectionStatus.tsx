import { useConnection } from "../state/connectionStore";
import { isError, isPulsing, statusLabel } from "../ipc";
import type { ConnStatus } from "../ipc";

interface Props {
  addr: string;
  status: ConnStatus;
  onOpenSettings(): void;
}

export function ConnectionStatusView({ addr, status, onOpenSettings }: Props) {
  const dotClass = isError(status)
    ? "bg-accent-red"
    : isPulsing(status)
      ? "bg-accent-green animate-pulse"
      : "bg-accent-green";
  return (
    <div className="flex items-center gap-1.5 min-w-0" aria-live="polite">
      <span
        aria-hidden
        className={`w-1.5 h-1.5 rounded-full shrink-0 ${dotClass}`}
      />
      <span className="truncate text-[10px] font-mono">
        {addr}
      </span>
      <span className="text-[10px] text-gray-400">·</span>
      <span className="text-[10px]">{statusLabel(status)}</span>
      {isError(status) && (
        <button
          type="button"
          onClick={onOpenSettings}
          className="ml-1 text-[10px] text-accent-blue hover:underline"
        >
          Open connection settings
        </button>
      )}
    </div>
  );
}

export function ConnectionStatus({ onOpenSettings }: { onOpenSettings(): void }) {
  const addr = useConnection((s) => s.addr);
  const status = useConnection((s) => s.status);
  return <ConnectionStatusView addr={addr} status={status} onOpenSettings={onOpenSettings} />;
}
