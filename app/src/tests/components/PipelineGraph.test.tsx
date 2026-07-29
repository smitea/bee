import { describe, it, expect, vi, beforeEach } from "vitest";
import { render, screen, fireEvent } from "@testing-library/react";

import type { PipelineDefinition } from "../../domain/pipeline";

const sample: PipelineDefinition = {
  id: 7,
  name: "btc_pipeline",
  input: {
    datasource: "binance",
    method: "subscribe",
    args: { symbol: "BTC/USDT", interval: "5m" },
    output: "ticks",
  },
  handlers: [
    { id: "h1", name: "compute_kline", params: { window: "5m", source: "ticks" }, upstream: ["ticks"] },
    { id: "h2", name: "aggregate", params: { method: "ohlc" }, upstream: ["h1"] },
  ],
  output: {
    adapter: "kafka",
    method: "publish",
    args: { topic: "out" },
    upstream: "h2",
  },
  crossPipelineRefs: [
    { upstreamPipelineName: "upstream_pipeline", upstreamPhaseId: "feed", downstreamPhaseId: "h1" },
  ],
};

beforeEach(() => {
  vi.resetModules();
});

describe("<PipelineGraph>", () => {
  it("renders the input node with datasource + method labels", async () => {
    const { PipelineGraph } = await import("../../domain/pipeline");
    render(
      <PipelineGraph
        pipeline={sample}
        onSelectInput={() => {}}
        onSelectOutput={() => {}}
        onSelectHandler={() => {}}
        onSelectCrossPipelineRef={() => {}}
      />,
    );
    expect(screen.getByText(/binance/)).toBeInTheDocument();
    expect(screen.getByText(/subscribe/)).toBeInTheDocument();
  });

  it("renders all handler nodes in order", async () => {
    const { PipelineGraph } = await import("../../domain/pipeline");
    render(
      <PipelineGraph
        pipeline={sample}
        onSelectInput={() => {}}
        onSelectOutput={() => {}}
        onSelectHandler={() => {}}
        onSelectCrossPipelineRef={() => {}}
      />,
    );
    expect(screen.getByText(/compute_kline/)).toBeInTheDocument();
    expect(screen.getByText(/aggregate/)).toBeInTheDocument();
  });

  it("renders the output node", async () => {
    const { PipelineGraph } = await import("../../domain/pipeline");
    render(
      <PipelineGraph
        pipeline={sample}
        onSelectInput={() => {}}
        onSelectOutput={() => {}}
        onSelectHandler={() => {}}
        onSelectCrossPipelineRef={() => {}}
      />,
    );
    expect(screen.getByText(/kafka/)).toBeInTheDocument();
    expect(screen.getByText(/publish/)).toBeInTheDocument();
  });

  it("renders cross-pipeline reference badges with target pipeline name", async () => {
    const { PipelineGraph } = await import("../../domain/pipeline");
    render(
      <PipelineGraph
        pipeline={sample}
        onSelectInput={() => {}}
        onSelectOutput={() => {}}
        onSelectHandler={() => {}}
        onSelectCrossPipelineRef={() => {}}
      />,
    );
    expect(screen.getByText(/upstream_pipeline/)).toBeInTheDocument();
  });

  it("clicking a handler node fires onSelectHandler with the handler id", async () => {
    const onSelectHandler = vi.fn();
    const { PipelineGraph } = await import("../../domain/pipeline");
    render(
      <PipelineGraph
        pipeline={sample}
        onSelectInput={() => {}}
        onSelectOutput={() => {}}
        onSelectHandler={onSelectHandler}
        onSelectCrossPipelineRef={() => {}}
      />,
    );
    fireEvent.click(screen.getByText(/compute_kline/));
    expect(onSelectHandler).toHaveBeenCalledWith("h1");
  });

  it("clicking the input node fires onSelectInput", async () => {
    const onSelectInput = vi.fn();
    const { PipelineGraph } = await import("../../domain/pipeline");
    render(
      <PipelineGraph
        pipeline={sample}
        onSelectInput={onSelectInput}
        onSelectOutput={() => {}}
        onSelectHandler={() => {}}
        onSelectCrossPipelineRef={() => {}}
      />,
    );
    fireEvent.click(screen.getByLabelText(/input node/i));
    expect(onSelectInput).toHaveBeenCalledTimes(1);
  });

  it("clicking the output node fires onSelectOutput", async () => {
    const onSelectOutput = vi.fn();
    const { PipelineGraph } = await import("../../domain/pipeline");
    render(
      <PipelineGraph
        pipeline={sample}
        onSelectInput={() => {}}
        onSelectOutput={onSelectOutput}
        onSelectHandler={() => {}}
        onSelectCrossPipelineRef={() => {}}
      />,
    );
    fireEvent.click(screen.getByLabelText(/output node/i));
    expect(onSelectOutput).toHaveBeenCalledTimes(1);
  });

  it("hovering a handler shows a tooltip with name + params", async () => {
    const { PipelineGraph } = await import("../../domain/pipeline");
    render(
      <PipelineGraph
        pipeline={sample}
        onSelectInput={() => {}}
        onSelectOutput={() => {}}
        onSelectHandler={() => {}}
        onSelectCrossPipelineRef={() => {}}
      />,
    );
    fireEvent.mouseOver(screen.getByText(/compute_kline/));
    const tip = await screen.findByRole("tooltip");
    expect(tip).toHaveTextContent(/compute_kline/);
    expect(tip).toHaveTextContent(/window/);
    expect(tip).toHaveTextContent(/5m/);
  });

  it("clicking a cross-pipeline ref fires onSelectCrossPipelineRef with the target", async () => {
    const onSelectCrossPipelineRef = vi.fn();
    const { PipelineGraph } = await import("../../domain/pipeline");
    render(
      <PipelineGraph
        pipeline={sample}
        onSelectInput={() => {}}
        onSelectOutput={() => {}}
        onSelectHandler={() => {}}
        onSelectCrossPipelineRef={onSelectCrossPipelineRef}
      />,
    );
    fireEvent.click(screen.getByLabelText(/cross-pipeline/i));
    expect(onSelectCrossPipelineRef).toHaveBeenCalledWith(
      expect.objectContaining({ upstreamPipelineName: "upstream_pipeline" }),
    );
  });
});
