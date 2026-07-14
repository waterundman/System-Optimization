import assert from "node:assert/strict";
import test from "node:test";
import type { ContextCandidate, ContextSource, KernelPorts } from "../src/index.ts";
import { ContextCompiler, DomainError, coreOperationProfiles } from "../src/index.ts";
import { ApproxTokenEstimator, FixedClock, SequentialIds, Sha256Hasher, createIntent } from "../../test-kit/src/index.ts";

function candidate(input: Partial<ContextCandidate> & Pick<ContextCandidate, "id" | "sourceRef" | "tier" | "content">): ContextCandidate {
  return {
    sourceHash: `sha256:${input.id}`,
    status: "canonical",
    authority: "source_derived",
    sensitivity: "local",
    renderMode: "verbatim",
    reasonCodes: ["TEST"],
    ...input,
  };
}

function source(id: string, candidates: readonly ContextCandidate[]): ContextSource {
  return { id, async collect() { return candidates; } };
}

function ports(sources: readonly ContextSource[]): KernelPorts {
  return {
    contextSources: sources,
    tokenizer: new ApproxTokenEstimator(),
    hasher: new Sha256Hasher(),
    clock: new FixedClock(),
    ids: new SequentialIds(),
  };
}

test("filters non-canonical and remote never-send candidates before scoring", async () => {
  const candidates = [
    candidate({ id: "target", sourceRef: "block:target", tier: "L0_TARGET", content: "目标正文", mandatory: true, authority: "user_confirmed" }),
    candidate({ id: "local", sourceRef: "block:local", tier: "L1_LOCAL", content: "相邻正文" }),
    candidate({ id: "archived", sourceRef: "fact:old", tier: "L3_KNOWLEDGE", content: "废弃设定", status: "archived" }),
    candidate({ id: "draft", sourceRef: "fact:draft", tier: "L3_KNOWLEDGE", content: "未确认推断", status: "draft" }),
    candidate({ id: "secret", sourceRef: "fact:secret", tier: "L3_KNOWLEDGE", content: "禁止外发", sensitivity: "never_send" }),
  ];
  const compiler = new ContextCompiler(ports([source("fixture", candidates)]));
  const packet = await compiler.compile({
    intent: createIntent(),
    profile: coreOperationProfiles.continue_scene,
    providerLocality: "remote",
    modelLimit: 1000,
    reservedOverhead: 100,
  });

  assert.deepEqual(packet.items.map((item) => item.sourceRef), ["block:target", "block:local"]);
  assert.deepEqual(
    Object.fromEntries(packet.exclusions.map((item) => [item.sourceRef, item.reason])),
    { "fact:draft": "DRAFT_NOT_SELECTED", "fact:old": "INELIGIBLE_STATUS", "fact:secret": "POLICY_DENIED" },
  );
});

test("is deterministic when inputs, ports and time are fixed", async () => {
  const candidates = [
    candidate({ id: "target", sourceRef: "block:target", tier: "L0_TARGET", content: "目标正文", mandatory: true }),
    candidate({ id: "fact", sourceRef: "fact:a", tier: "L3_KNOWLEDGE", content: "角色左眼失明", signals: { relevance: 0.9 } }),
  ];
  const request = {
    intent: createIntent(),
    profile: coreOperationProfiles.continue_scene,
    providerLocality: "local" as const,
    modelLimit: 1000,
    reservedOverhead: 100,
  };
  const first = await new ContextCompiler(ports([source("fixture", candidates)])).compile(request);
  const second = await new ContextCompiler(ports([source("fixture", [...candidates].reverse())])).compile(request);
  assert.deepEqual(first, second);
  assert.match(first.packetHash, /^sha256:[a-f0-9]{64}$/);
});

test("fails instead of silently dropping mandatory context", async () => {
  const compiler = new ContextCompiler(ports([
    source("fixture", [
      candidate({ id: "target", sourceRef: "block:target", tier: "L0_TARGET", content: "非常长的目标正文".repeat(100), mandatory: true }),
    ]),
  ]));

  await assert.rejects(
    compiler.compile({
      intent: createIntent({ output: { kind: "patch_proposal", maxTokens: 80 } }),
      profile: coreOperationProfiles.continue_scene,
      providerLocality: "local",
      modelLimit: 100,
      reservedOverhead: 10,
    }),
    (error: unknown) => error instanceof DomainError && error.code === "CTX_BUDGET_EXCEEDED",
  );
});

test("deduplicates the same source revision and render mode", async () => {
  const duplicate = candidate({ id: "same", sourceRef: "block:same", sourceHash: "sha256:same", tier: "L1_LOCAL", content: "相邻段落" });
  const compiler = new ContextCompiler(ports([
    source("a", [candidate({ id: "target", sourceRef: "block:target", tier: "L0_TARGET", content: "目标", mandatory: true }), duplicate]),
    source("b", [{ ...duplicate, id: "same-again" }]),
  ]));
  const packet = await compiler.compile({
    intent: createIntent(),
    profile: coreOperationProfiles.continue_scene,
    providerLocality: "local",
    modelLimit: 1000,
    reservedOverhead: 100,
  });
  assert.equal(packet.items.filter((item) => item.sourceRef === "block:same").length, 1);
  assert.equal(packet.exclusions.some((item) => item.reason === "DUPLICATE"), true);
});

