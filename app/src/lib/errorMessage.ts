export function errorMessage(err: unknown): string {
  if (err == null) return String(err);
  if (typeof err === "string") return err;
  if (err instanceof Error) return err.message || err.name || "Error";
  if (typeof err === "object") {
    const obj = err as { message?: unknown; toString?: () => string };
    if (typeof obj.message === "string" && obj.message.length > 0) return obj.message;
    const toString = obj.toString;
    if (typeof toString === "function" && toString !== Object.prototype.toString) {
      return toString.call(err);
    }
    try {
      return JSON.stringify(err);
    } catch {
      return Object.prototype.toString.call(err);
    }
  }
  return String(err);
}