import "@testing-library/jest-dom/vitest";
import { afterEach, vi } from "vitest";
import { cleanup } from "@testing-library/react";
import * as React from "react";

class ResizeObserverMock {
  observe() {}
  unobserve() {}
  disconnect() {}
}

if (typeof globalThis.ResizeObserver === "undefined") {
  globalThis.ResizeObserver = ResizeObserverMock as unknown as typeof ResizeObserver;
}

if (typeof globalThis.matchMedia === "undefined") {
  globalThis.matchMedia = ((query: string) => ({
    matches: false,
    media: query,
    onchange: null,
    addListener: () => {},
    removeListener: () => {},
    addEventListener: () => {},
    removeEventListener: () => {},
    dispatchEvent: () => false,
  })) as unknown as typeof window.matchMedia;
}

if (typeof globalThis.URL.createObjectURL === "undefined") {
  globalThis.URL.createObjectURL = (() => "blob:mock") as typeof URL.createObjectURL;
  globalThis.URL.revokeObjectURL = (() => undefined) as typeof URL.revokeObjectURL;
}

const makeStub = (name: string) => {
  if (name === "addEdge" || name === "applyEdgeChanges" || name === "applyNodeChanges") {
    return vi.fn();
  }
  return (props: { children?: React.ReactNode }) =>
    React.createElement(
      "div",
      { "data-testid": `xyflow-${name.toLowerCase()}` },
      props.children,
    );
};

vi.mock("@xyflow/react", () => {
  const stub: Record<string, unknown> = {};
  for (const name of [
    "ReactFlow",
    "Background",
    "Controls",
    "MiniMap",
    "addEdge",
    "applyEdgeChanges",
    "applyNodeChanges",
  ]) {
    stub[name] = makeStub(name);
  }
  return stub;
});

afterEach(() => cleanup());