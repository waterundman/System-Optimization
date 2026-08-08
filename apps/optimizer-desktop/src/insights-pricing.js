export const PRICING_TABLE = Object.freeze({
  deepseek: { inputPer1k: 0.002, outputPer1k: 0.006, cachedPer1k: 0.001 },
  qwen: { inputPer1k: 0.004, outputPer1k: 0.012, cachedPer1k: 0.002 },
  kimi: { inputPer1k: 0.006, outputPer1k: 0.018, cachedPer1k: 0.003 },
  minimax: { inputPer1k: 0.005, outputPer1k: 0.015, cachedPer1k: 0.002 },
  ollama: { inputPer1k: 0, outputPer1k: 0, cachedPer1k: 0 },
});

export function estimateFees(recentRuns) {
  const buckets = {};
  if (!Array.isArray(recentRuns)) return buckets;
  for (const run of recentRuns) {
    if (!run || typeof run !== "object") continue;
    const providerId = run.providerId;
    if (!providerId || typeof providerId !== "string") continue;
    const pricing = PRICING_TABLE[providerId];
    if (!pricing) continue;
    const bucket = buckets[providerId] ?? { inputTokens: 0, outputTokens: 0, cachedInputTokens: 0, fee: 0 };

    const hasV2 = (run.inputTokens !== null && run.inputTokens !== undefined && typeof run.inputTokens === "number")
               || (run.outputTokens !== null && run.outputTokens !== undefined && typeof run.outputTokens === "number");
    if (hasV2) {
      const inputTokens = typeof run.inputTokens === "number" ? run.inputTokens : 0;
      const outputTokens = typeof run.outputTokens === "number" ? run.outputTokens : 0;
      const cachedInputTokens = typeof run.cachedInputTokens === "number" ? run.cachedInputTokens : 0;
      bucket.inputTokens += inputTokens;
      bucket.outputTokens += outputTokens;
      bucket.cachedInputTokens += cachedInputTokens;
      bucket.fee += (inputTokens / 1000) * pricing.inputPer1k
                 + (outputTokens / 1000) * pricing.outputPer1k
                 + (cachedInputTokens / 1000) * pricing.cachedPer1k;
    } else {
      const totalTokens = run.totalTokens;
      if (totalTokens === null || totalTokens === undefined || typeof totalTokens !== "number") continue;
      bucket.inputTokens += totalTokens;
      const avgPer1k = (pricing.inputPer1k + pricing.outputPer1k) / 2;
      bucket.fee += (totalTokens / 1000) * avgPer1k;
    }
    buckets[providerId] = bucket;
  }
  return buckets;
}
