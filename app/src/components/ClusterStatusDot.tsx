import { clusterStatusColor } from "../domain/clusterStatusColor";
import type { ConnStatus } from "../ipc/shared";

interface Props {
  profileAddr: string;
  activeAddr: string;
  status: ConnStatus;
  size?: number;
}

export function ClusterStatusDot({ profileAddr, activeAddr, status, size = 8 }: Props) {
  const kind = clusterStatusColor(profileAddr, activeAddr, status.kind);
  const cls =
    kind === "green"
      ? "bg-accent-green"
      : kind === "amber"
        ? "bg-accent-orange"
        : "bg-accent-red";
  const title = `${profileAddr} — ${kind === "green" ? "active" : kind === "amber" ? "saved" : "error"}`;
  return (
    <span
      role="status"
      aria-label={title}
      title={title}
      data-status={kind}
      className={`inline-block rounded-full ${cls}`}
      style={{ width: size, height: size }}
    />
  );
}