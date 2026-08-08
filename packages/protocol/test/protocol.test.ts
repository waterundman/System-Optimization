import assert from "node:assert/strict";
import test from "node:test";
import {
  validateModelProviderConfiguration,
  validateOperationIntent,
  validatePatchProposal,
} from "../src/index.ts";

const valid = {
  schemaVersion: 1,
  id: "intent-1",
  projectId: "project-1",
  baseCommitId: "commit-1",
  type: "polish",
  strength: "low",
  target: {
    documentId: "document-1",
    blockId: "block-1",
    baseRevision: 2,
    baseHash: "sha256:base",
    from: { blockId: "block-1", offset: 0 },
    to: { blockId: "block-1", offset: 4 },
  },
  constraints: [],
  output: { kind: "patch_proposal", maxTokens: 500 },
  createdAt: "2026-07-14T00:00:00.000Z",
};

function constraintEntry(overrides: Readonly<Record<string, unknown>> = {}) {
  return { severity: "hard", rule: "保持原文语言", ...overrides };
}

test("validates a supported OperationIntent", () => {
  const result = validateOperationIntent(valid);
  assert.equal(result.ok, true);
});

test("rejects an invalid target and output budget", () => {
  const result = validateOperationIntent({
    ...valid,
    target: { ...valid.target, baseRevision: -1 },
    output: { ...valid.output, maxTokens: 0 },
  });
  assert.equal(result.ok, false);
  if (!result.ok) {
    assert.deepEqual(
      result.issues.map((issue) => issue.path),
      ["$.target.baseRevision", "$.output.maxTokens"],
    );
  }
});

test("validates operation constraints against the schema shape", () => {
  const constraint = { severity: "hard", rule: "保持原文语言", sourceRef: "fact:style" };
  assert.equal(validateOperationIntent({
    ...valid,
    constraints: [constraint, { severity: "soft", rule: "尽量精简" }],
    userInstruction: "改写这段",
  }).ok, true);

  // Existing legal inputs (non-string or absent userInstruction) still pass.
  assert.equal(validateOperationIntent({
    ...valid,
    constraints: [],
    userInstruction: "",
  }).ok, true);
});

test("rejects malformed constraints with per-item paths", () => {
  const invalid = validateOperationIntent({
    ...valid,
    constraints: [
      constraintEntry({ severity: "urgent", rule: "必须", sourceRef: "" }),
      constraintEntry({ rule: "" }),
      constraintEntry({ severity: "hard", rule: "ok", injected: true }),
      null,
    ],
    userInstruction: "x".repeat(10_001),
  });
  assert.equal(invalid.ok, false);
  if (!invalid.ok) {
    assert.deepEqual(
      invalid.issues.map((issue) => issue.path),
      [
        "$.constraints[0].severity",
        "$.constraints[0].sourceRef",
        "$.constraints[1].rule",
        "$.constraints[2].injected",
        "$.constraints[3]",
        "$.userInstruction",
      ],
    );
  }
});

test("rejects constraint overruns and user instruction length limits", () => {
  const tooMany = validateOperationIntent({
    ...valid,
    constraints: Array.from({ length: 257 }, (_, index) => ({
      severity: "hard",
      rule: `rule-${index}`,
    })),
  });
  assert.equal(tooMany.ok, false);
  if (tooMany.ok) return;
  assert.equal(
    tooMany.issues.some((issue) => issue.path === "$.constraints"
      && issue.message.includes("256")),
    true,
  );

  const longRule = validateOperationIntent({
    ...valid,
    constraints: [{ severity: "hard", rule: "长".repeat(10_001) }],
  });
  assert.equal(longRule.ok, false);
  if (longRule.ok) return;
  assert.equal(
    longRule.issues.some((issue) => issue.path === "$.constraints[0].rule"),
    true,
  );
});

test("validates immutable PatchProposal hunk baselines", () => {
  const proposal = {
    schemaVersion: 2,
    id: "proposal-1",
    operationRunId: "run-1",
    baseCommitId: "commit-1",
    target: valid.target,
    hunks: [{
      id: "hunk-1",
      from: { blockId: "block-1", offset: 1 },
      to: { blockId: "block-1", offset: 3 },
      original: "原文",
      replacement: "修改",
      granularity: "token",
    }],
    warnings: [],
    status: "review",
    createdAt: "2026-07-14T00:00:00.000Z",
    proposalHash: `sha256:${"0".repeat(64)}`,
  };
  assert.equal(validatePatchProposal(proposal).ok, true);

  const invalid = validatePatchProposal({
    ...proposal,
    hunks: [{ ...proposal.hunks[0], original: "长度不匹配" }],
  });
  assert.equal(invalid.ok, false);
  if (!invalid.ok) {
    assert.equal(invalid.issues.some((issue) => issue.path === "$.hunks[0].original"), true);
  }

  const outsideTarget = validatePatchProposal({
    ...proposal,
    hunks: [{
      ...proposal.hunks[0],
      from: { blockId: "block-1", offset: 5 },
      to: { blockId: "block-1", offset: 5 },
      original: "",
    }],
  });
  assert.equal(outsideTarget.ok, false);
  if (!outsideTarget.ok) {
    assert.equal(
      outsideTarget.issues.some((issue) => issue.message.includes("within the proposal target")),
      true,
    );
  }
});

test("persists provider settings through credential references, never API keys", () => {
  const configuration = {
    schemaVersion: 1,
    id: "provider-deepseek-default",
    providerId: "deepseek",
    enabled: true,
    defaultModel: "deepseek-v4-flash",
    credentialRef: "secret://providers/deepseek/default",
    defaultTimeoutMs: 60_000,
    maxRequestBytes: 16 * 1024 * 1024,
    updatedAt: "2026-07-14T00:00:00.000Z",
  };
  assert.equal(validateModelProviderConfiguration(configuration).ok, true);
  assert.equal(validateModelProviderConfiguration({
    ...configuration,
    id: "provider-ollama-default",
    providerId: "ollama",
    defaultModel: "qwen3:8b",
    credentialRef: "secret://providers/ollama/default",
  }).ok, true);

  const endpointId = `endpoint-${"a".repeat(32)}`;
  const compatible = {
    ...configuration,
    id: `provider-openai-compatible-${endpointId}`,
    providerId: "openai_compatible",
    defaultModel: "acme-writer",
    credentialRef: `secret://providers/openai-compatible/${endpointId}`,
    openaiCompatible: {
      endpointId,
      endpointRevision: 1,
      jsonObject: false,
      streamUsage: false,
      maxOutputTokenField: "max_tokens",
    },
  };
  assert.equal(validateModelProviderConfiguration(compatible).ok, true);
  const unboundCompatible = validateModelProviderConfiguration({
    ...compatible,
    openaiCompatible: undefined,
  });
  assert.equal(unboundCompatible.ok, false);
  const capabilityInjection = validateModelProviderConfiguration({
    ...compatible,
    openaiCompatible: { ...compatible.openaiCompatible, baseUrl: "https://attacker.invalid" },
  });
  assert.equal(capabilityInjection.ok, false);
  assert.equal(validateModelProviderConfiguration({
    ...compatible,
    id: "provider-forged",
  }).ok, false);
  assert.equal(validateModelProviderConfiguration({
    ...compatible,
    credentialRef: "secret://providers/openai-compatible/endpoint-bbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbb",
  }).ok, false);

  const leaked = validateModelProviderConfiguration({
    ...configuration,
    apiKey: "must-not-be-persisted",
  });
  assert.equal(leaked.ok, false);
  if (!leaked.ok) {
    assert.equal(leaked.issues.some((issue) => issue.path === "$.apiKey"), true);
  }
});
