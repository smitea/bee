import { describe, it, expect } from "vitest";
import { parsePipeline } from "../../domain/pipeline";

const base = {
  id: 1,
  name: "empty_pipeline",
  updated_at: 1700000000,
};

describe("parsePipeline", () => {
  it("uses 未配置 as the input placeholder when dag_json omits input", () => {
    const parsed = parsePipeline({
      ...base,
      dag_json: JSON.stringify({ handlers: [], output: { adapter: "kafka", method: "publish", args: {}, upstream: "h1" } }),
    });
    expect(parsed.input.datasource).toBe("未配置");
    expect(parsed.input.method).toBe("subscribe");
  });

  it("uses 未配置 as the output placeholder when dag_json omits output", () => {
    const parsed = parsePipeline({
      ...base,
      dag_json: JSON.stringify({ input: { datasource: "binance", method: "subscribe", args: {}, output: "ticks" }, handlers: [] }),
    });
    expect(parsed.output.adapter).toBe("未配置");
    expect(parsed.output.method).toBe("emit");
  });

  it("uses 未配置 for both placeholders when dag_json is empty object", () => {
    const parsed = parsePipeline({ ...base, dag_json: "{}" });
    expect(parsed.input.datasource).toBe("未配置");
    expect(parsed.output.adapter).toBe("未配置");
  });

  it("uses 未配置 placeholders when dag_json is malformed", () => {
    const parsed = parsePipeline({ ...base, dag_json: "not json" });
    expect(parsed.input.datasource).toBe("未配置");
    expect(parsed.output.adapter).toBe("未配置");
    expect(parsed.handlers).toEqual([]);
    expect(parsed.crossPipelineRefs).toEqual([]);
  });

  it("preserves a real datasource when dag_json provides one", () => {
    const parsed = parsePipeline({
      ...base,
      dag_json: JSON.stringify({
        input: { datasource: "binance", method: "subscribe", args: {}, output: "ticks" },
        handlers: [],
        output: { adapter: "kafka", method: "publish", args: {}, upstream: "h1" },
      }),
    });
    expect(parsed.input.datasource).toBe("binance");
    expect(parsed.output.adapter).toBe("kafka");
  });

  it("does not emit the old (none) placeholder anywhere", () => {
    const parsed = parsePipeline({ ...base, dag_json: "{}" });
    expect(JSON.stringify(parsed)).not.toContain("(none)");
  });
});
