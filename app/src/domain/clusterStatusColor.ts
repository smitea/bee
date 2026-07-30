export type ClusterStatusKind = "Connected" | "Connecting" | "Disconnected" | "Error";

export function clusterStatusColor(
  profileAddr: string,
  activeAddr: string,
  statusKind: ClusterStatusKind,
): "green" | "amber" | "red" {
  const norm = (a: string) => a.trim();
  if (norm(profileAddr) !== norm(activeAddr)) return "amber";
  if (statusKind === "Connected" || statusKind === "Connecting") return "green";
  return "red";
}