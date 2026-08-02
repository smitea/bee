import { describe, expect, it } from "vitest";

import { errorMessage } from "../../lib/errorMessage";

describe("errorMessage", () => {
  it("returns string errors verbatim", () => {
    expect(errorMessage("boom")).toBe("boom");
    expect(errorMessage("")).toBe("");
  });

  it("extracts message from Error instances", () => {
    expect(errorMessage(new Error("disk full"))).toBe("disk full");
    expect(errorMessage(new TypeError("bad arg"))).toBe("bad arg");
  });

  it("extracts message property from plain objects (Tauri 2 errors)", () => {
    expect(errorMessage({ message: "permission denied" })).toBe("permission denied");
  });

  it("falls back to JSON.stringify for unknown objects without message", () => {
    const result = errorMessage({ code: 42, reason: "nope" });
    expect(result).toBe('{"code":42,"reason":"nope"}');
  });

  it("returns [object Object] only when JSON.stringify throws", () => {
    const circular: Record<string, unknown> = {};
    circular.self = circular;
    const result = errorMessage(circular);
    expect(result).toBe("[object Object]");
  });

  it("handles null and undefined safely", () => {
    expect(errorMessage(null)).toBe("null");
    expect(errorMessage(undefined)).toBe("undefined");
  });

  it("handles numbers and booleans via String()", () => {
    expect(errorMessage(42)).toBe("42");
    expect(errorMessage(false)).toBe("false");
    expect(errorMessage(true)).toBe("true");
  });

  it("uses custom toString when available", () => {
    const obj = {
      toString: () => "custom string",
    };
    expect(errorMessage(obj)).toBe("custom string");
  });

  it("falls back to empty message Error to Error name", () => {
    const e = new Error("");
    expect(errorMessage(e)).toMatch(/Error/);
  });
});