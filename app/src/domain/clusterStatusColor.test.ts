import { describe, it, expect } from "vitest";

import { clusterStatusColor } from "./clusterStatusColor";

describe("clusterStatusColor", () => {
  it("returns green when the profile addr matches the active connected addr", () => {
    expect(clusterStatusColor("127.0.0.1:9999", "127.0.0.1:9999", "Connected")).toBe("green");
    expect(clusterStatusColor("127.0.0.1:9999", "127.0.0.1:9999", "Connecting")).toBe("green");
  });

  it("returns red when the profile addr matches but the connection is errored", () => {
    expect(clusterStatusColor("127.0.0.1:9999", "127.0.0.1:9999", "Error")).toBe("red");
    expect(clusterStatusColor("127.0.0.1:9999", "127.0.0.1:9999", "Disconnected")).toBe("red");
  });

  it("returns amber when the profile addr differs from the active addr (saved but not active)", () => {
    expect(clusterStatusColor("127.0.0.1:9999", "10.0.0.1:9999", "Connected")).toBe("amber");
    expect(clusterStatusColor("127.0.0.1:9999", "10.0.0.1:9999", "Error")).toBe("amber");
  });

  it("is case- and whitespace-insensitive about addr matching", () => {
    expect(clusterStatusColor(" 127.0.0.1:9999 ", "127.0.0.1:9999", "Connected")).toBe("green");
  });
});