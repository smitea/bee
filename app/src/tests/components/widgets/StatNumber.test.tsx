import { describe, expect, it } from "vitest";
import { render, screen } from "@testing-library/react";

import { StatNumber } from "../../../components/widgets/StatNumber";

function trendDivByArrow(arrow: string): HTMLElement {
  const text = screen.getByText(arrow);
  let node: Element | null = text;
  while (node && node.parentElement) {
    if (node.tagName === "DIV" && (node as HTMLElement).className.includes("font-mono")) {
      return node as HTMLElement;
    }
    node = node.parentElement;
  }
  throw new Error(`Could not find trend div containing arrow '${arrow}'`);
}

describe("<StatNumber>", () => {
  it("mounts with a sample value and label", () => {
    render(<StatNumber value={42} label="Active Jobs" />);
    expect(screen.getByText("42")).toBeInTheDocument();
    expect(screen.getByText("Active Jobs")).toBeInTheDocument();
  });

  it("renders a string value verbatim (e.g. placeholder / loading)", () => {
    render(<StatNumber value="Loading…" label="Active Jobs" />);
    expect(screen.getByText("Loading…")).toBeInTheDocument();
    expect(screen.queryByText("42")).toBeNull();
  });

  it("renders the unit next to the value when provided", () => {
    render(<StatNumber value={3} label="Running / Failed" unit="/0" />);
    expect(screen.getByText("3")).toBeInTheDocument();
    expect(screen.getByText("/0")).toBeInTheDocument();
    expect(screen.getByText("Running / Failed")).toBeInTheDocument();
  });

  it("renders an up-trend delta with the green color and up arrow", () => {
    render(<StatNumber value={100} label="CPU" trend="up" />);
    expect(screen.getByText("▲")).toBeInTheDocument();
    const trend = trendDivByArrow("▲");
    expect(trend.className).toMatch(/text-accent-green/);
  });

  it("renders a down-trend delta with the red color and down arrow", () => {
    render(<StatNumber value={10} label="Latency" trend="down" />);
    expect(screen.getByText("▼")).toBeInTheDocument();
    const trend = trendDivByArrow("▼");
    expect(trend.className).toMatch(/text-accent-red/);
  });

  it("renders a flat trend delta with the muted gray color and em-dash", () => {
    render(<StatNumber value={5} label="Jobs" trend="flat" />);
    expect(screen.getByText("—")).toBeInTheDocument();
    const trend = trendDivByArrow("—");
    expect(trend.className).toMatch(/text-gray-400/);
  });

  it("omits the trend row when no trend prop is provided", () => {
    render(<StatNumber value={42} label="Active Jobs" />);
    expect(screen.queryByText("▲")).toBeNull();
    expect(screen.queryByText("▼")).toBeNull();
    expect(screen.queryByText("—")).toBeNull();
  });

  it("applies tabular-nums formatting to the value", () => {
    render(<StatNumber value={1234} label="Events" />);
    const value = screen.getByText("1234");
    const valueDiv = value.closest("div");
    expect(valueDiv?.className).toMatch(/tabular-nums/);
  });

  it("renders an error-state placeholder without throwing when given a non-finite number", () => {
    render(<StatNumber value={Number.NaN} label="Latency" unit="ms" />);
    expect(screen.getByText("NaN")).toBeInTheDocument();
    expect(screen.getByText("Latency")).toBeInTheDocument();
  });

  it("renders the loading placeholder without a trend indicator", () => {
    render(<StatNumber value="—" label="Loading…" />);
    expect(screen.getByText("—")).toBeInTheDocument();
    expect(screen.getByText("Loading…")).toBeInTheDocument();
  });
});
