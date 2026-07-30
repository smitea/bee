import { beforeEach, describe, expect, it, vi } from "vitest";
import { render, screen } from "@testing-library/react";

interface EChartsMockProps {
  option?: Record<string, unknown>;
  style?: React.CSSProperties;
  opts?: { renderer?: string };
}

vi.mock("echarts-for-react", () => ({
  default: (props: EChartsMockProps) => (
    <div
      data-testid="echarts-mock"
      data-option={JSON.stringify(props.option ?? {})}
      data-renderer={props.opts?.renderer ?? "svg"}
    />
  ),
}));

import { BarChart } from "../../../components/widgets/BarChart";

const sampleData = [
  { label: "queued", value: 1 },
  { label: "running", value: 3 },
  { label: "historical", value: 12 },
  { label: "failed", value: 0 },
];

describe("<BarChart>", () => {
  beforeEach(() => {
    vi.clearAllMocks();
  });

  it("mounts with sample data and renders an echarts canvas container", () => {
    const { container } = render(<BarChart data={sampleData} />);
    expect(container.querySelector('[data-testid="echarts-mock"]')).toBeInTheDocument();
    expect(screen.queryByText(/no data/i)).toBeNull();
  });

  it("asks echarts for the canvas renderer (jsdom does not implement canvas)", () => {
    const { container } = render(<BarChart data={sampleData} />);
    const el = container.querySelector('[data-testid="echarts-mock"]');
    expect(el?.getAttribute("data-renderer")).toBe("canvas");
  });

  it("forwards labels and values into the echarts option payload", () => {
    const { container } = render(<BarChart data={sampleData} color="#10b981" />);
    const el = container.querySelector('[data-testid="echarts-mock"]');
    const raw = el?.getAttribute("data-option") ?? "{}";
    const option = JSON.parse(raw) as {
      xAxis?: { data?: string[] };
      series?: Array<{ data?: number[]; itemStyle?: { color?: string } }>;
    };
    expect(option.xAxis?.data).toEqual(["queued", "running", "historical", "failed"]);
    expect(option.series?.[0]?.data).toEqual([1, 3, 12, 0]);
    expect(option.series?.[0]?.itemStyle?.color).toBe("#10b981");
  });

  it("renders the optional title into the echarts option when provided", () => {
    const { container } = render(<BarChart data={sampleData} title="Job mix" />);
    const el = container.querySelector('[data-testid="echarts-mock"]');
    const option = JSON.parse((el?.getAttribute("data-option") ?? "{}") as string) as {
      title?: { text?: string };
    };
    expect(option.title?.text).toBe("Job mix");
  });

  it("falls back to an empty-state placeholder when no data is provided", () => {
    render(<BarChart data={[]} />);
    expect(screen.getByText(/no data/i)).toBeInTheDocument();
    expect(screen.queryByTestId("echarts-mock")).toBeNull();
  });

  it("disposes cleanly on unmount when data is provided", () => {
    const { unmount } = render(<BarChart data={sampleData} />);
    expect(() => unmount()).not.toThrow();
  });

  it("disposes cleanly on unmount when data is empty", () => {
    const { unmount } = render(<BarChart data={[]} />);
    expect(() => unmount()).not.toThrow();
  });
});
