export type ConnStatus =
  | { kind: "Connected" }
  | { kind: "Connecting" }
  | { kind: "Disconnected" }
  | { kind: "Error"; reason: string };

export interface StateView {
  addr: string;
  status: ConnStatus;
}

export function statusLabel(s: ConnStatus): string {
  switch (s.kind) {
    case "Connected":
      return "Connected";
    case "Connecting":
      return "Connecting";
    case "Disconnected":
      return "Disconnected";
    case "Error":
      return `Error: ${s.reason}`;
  }
}

export function isPulsing(s: ConnStatus): boolean {
  return s.kind === "Connecting";
}

export function isError(s: ConnStatus): boolean {
  return s.kind === "Error" || s.kind === "Disconnected";
}
