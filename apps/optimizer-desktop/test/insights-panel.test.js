import assert from "node:assert/strict";
import test from "node:test";

import { PRICING_TABLE, estimateFees } from "../src/insights-pricing.js";

test("PRICING_TABLE covers five providers and ollama is priced at zero", () => {
  const expectedProviders = ["deepseek", "qwen", "kimi", "minimax", "ollama"];
  for (const providerId of expectedProviders) {
    assert.ok(Object.prototype.hasOwnProperty.call(PRICING_TABLE, providerId), `missing ${providerId}`);
    const pricing = PRICING_TABLE[providerId];
    assert.equal(typeof pricing.inputPer1k, "number");
    assert.equal(typeof pricing.outputPer1k, "number");
    assert.equal(typeof pricing.cachedPer1k, "number");
  }
  assert.deepEqual(PRICING_TABLE.ollama, { inputPer1k: 0, outputPer1k: 0, cachedPer1k: 0 });
  assert.ok(Object.isFrozen(PRICING_TABLE), "PRICING_TABLE should be frozen");
});

test("estimateFees returns an empty object for an empty array", () => {
  assert.deepEqual(estimateFees([]), {});
});

test("estimateFees skips runs with null totalTokens", () => {
  const runs = [
    { runId: "run-1", providerId: "deepseek", state: "accepted", totalTokens: null },
    { runId: "run-2", providerId: "qwen", state: "accepted", totalTokens: 500 },
  ];
  const fees = estimateFees(runs);
  assert.ok(!Object.prototype.hasOwnProperty.call(fees, "deepseek"), "deepseek should be skipped");
  assert.ok(Object.prototype.hasOwnProperty.call(fees, "qwen"), "qwen should be present");
  assert.equal(fees.qwen.inputTokens, 500);
});

test("estimateFees aggregates tokens for the same providerId", () => {
  const runs = [
    { runId: "run-1", providerId: "deepseek", state: "accepted", totalTokens: 1000 },
    { runId: "run-2", providerId: "deepseek", state: "rejected", totalTokens: 2000 },
    { runId: "run-3", providerId: "qwen", state: "accepted", totalTokens: 4000 },
  ];
  const fees = estimateFees(runs);
  assert.equal(fees.deepseek.inputTokens, 3000);
  assert.equal(fees.deepseek.outputTokens, 0);
  const expectedDeepseekFee = (3000 / 1000) * ((0.002 + 0.006) / 2);
  assert.equal(fees.deepseek.fee, expectedDeepseekFee);
  assert.equal(fees.qwen.inputTokens, 4000);
  const expectedQwenFee = (4000 / 1000) * ((0.004 + 0.012) / 2);
  assert.equal(fees.qwen.fee, expectedQwenFee);
});

test("estimateFees returns zero fee for ollama provider", () => {
  const runs = [
    { runId: "run-1", providerId: "ollama", state: "accepted", totalTokens: 5000 },
    { runId: "run-2", providerId: "ollama", state: "accepted", totalTokens: 3000 },
  ];
  const fees = estimateFees(runs);
  assert.equal(fees.ollama.inputTokens, 8000);
  assert.equal(fees.ollama.fee, 0);
});

test("T06: estimateFees 精确公式 - input/output/cached 分别按对应单价计费", () => {
  const runs = [
    { providerId: "deepseek", inputTokens: 1000, outputTokens: 500, cachedInputTokens: 200, totalTokens: 1700 },
  ];
  const result = estimateFees(runs);
  const bucket = result.deepseek;
  assert.equal(bucket.inputTokens, 1000);
  assert.equal(bucket.outputTokens, 500);
  assert.equal(bucket.cachedInputTokens, 200);
  // 1000/1000 * 0.002 + 500/1000 * 0.006 + 200/1000 * 0.001 = 0.002 + 0.003 + 0.0002 = 0.0052
  assert.ok(Math.abs(bucket.fee - 0.0052) < 1e-9, `expected 0.0052, got ${bucket.fee}`);
});

test("T07: estimateFees v1 fallback - 仅 totalTokens 时回退到平均单价", () => {
  const runs = [
    { providerId: "deepseek", totalTokens: 1000 },
  ];
  const result = estimateFees(runs);
  const bucket = result.deepseek;
  assert.equal(bucket.inputTokens, 1000);
  assert.equal(bucket.outputTokens, 0);
  // 1000/1000 * (0.002 + 0.006) / 2 = 0.004
  assert.ok(Math.abs(bucket.fee - 0.004) < 1e-9, `expected 0.004, got ${bucket.fee}`);
});

test("T08: estimateFees cachedInputTokens 按 cachedPer1k 计费（而非 inputPer1k）", () => {
  const runs = [
    { providerId: "deepseek", inputTokens: 0, outputTokens: 0, cachedInputTokens: 1000, totalTokens: 1000 },
  ];
  const result = estimateFees(runs);
  const bucket = result.deepseek;
  // 1000/1000 * 0.001 (cachedPer1k) = 0.001，不是 0.002 (inputPer1k)
  assert.ok(Math.abs(bucket.fee - 0.001) < 1e-9, `expected 0.001, got ${bucket.fee}`);
});
