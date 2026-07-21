import assert from "node:assert/strict";
import { readFile } from "node:fs/promises";
import test from "node:test";
import type {
  BlockId,
  CommitId,
  ContextPacket,
  ContextPacketId,
  OperationRunId,
  OperationPersistenceBundleV1,
  PatchProposal,
  PatchProposalId,
  ProjectId,
} from "../../protocol/src/index.ts";
import { ProviderError } from "../../model-gateway/src/index.ts";
import { Sha256Hasher, createIntent } from "../../test-kit/src/index.ts";
import {
  OperationExecutionError,
  buildFailedPersistenceBundle,
  buildSuccessfulPersistenceBundle,
  serializeOperationPersistenceBundle,
  type OperationExecutionResult,
} from "../src/index.ts";

const at = "2026-07-15T00:00:00.000Z";

test("shares the versioned persistence fixture with the Rust host", async () => {
  const fixtureUrl = new URL(
    "../../protocol/fixtures/operation-persistence-bundle.v1.json",
    import.meta.url,
  );
  const fixture = JSON.parse(await readFile(fixtureUrl, "utf8")) as OperationPersistenceBundleV1;
  assert.equal(fixture.schemaVersion, 1);
  assert.equal(fixture.run.id, "run-fixture-1");
  assert.equal(fixture.artifact?.kind, "patch_proposal");
  assert.equal(fixture.lifecycleEvents.at(-1)?.toState, fixture.run.state);
  assert.equal(fixture.attempts?.length, 2);
  assert.equal(fixture.attempts?.[0]?.retryDelayMs, 800);
});

function contextPacket(): ContextPacket {
  return {
    schemaVersion: 1,
    id: "context-persist-1" as ContextPacketId,
    operationIntentId: createIntent().id,
    projectId: "project-1" as ProjectId,
    baseCommitId: "commit-1" as CommitId,
    providerLocality: "remote",
    items: [],
    exclusions: [],
    budget: {
      modelLimit: 8_000,
      reservedOutput: 200,
      reservedOverhead: 800,
      inputBudget: 7_000,
      estimatedInput: 10,
      tokenizer: "test",
    },
    compilerVersion: "1.0.0",
    operationProfileVersion: "1.0.0",
    compiledAt: at,
    packetHash: "sha256:context",
  };
}

function history(final: "review" | "failed") {
  return final === "review"
    ? [
        { from: "draft", to: "compiling", occurredAt: at },
        { from: "compiling", to: "preflight", occurredAt: at },
        { from: "preflight", to: "queued", occurredAt: at },
        { from: "queued", to: "streaming", occurredAt: at },
        { from: "streaming", to: "validating", occurredAt: at },
        { from: "validating", to: "review", occurredAt: at },
      ] as const
    : [
        { from: "draft", to: "compiling", occurredAt: at },
        { from: "compiling", to: "failed", occurredAt: at, reason: "ProviderError" },
      ] as const;
}

test("maps a successful patch result into a deterministic persistence bundle", async () => {
  const intent = createIntent();
  const blockId = intent.target.blockId as BlockId;
  const proposal: PatchProposal = {
    schemaVersion: 2,
    id: "proposal-persist-1" as PatchProposalId,
    operationRunId: "run-persist-1" as OperationRunId,
    baseCommitId: intent.baseCommitId,
    target: intent.target,
    hunks: [{
      id: "h1",
      from: { blockId, offset: 0 },
      to: { blockId, offset: 1 },
      original: "夜",
      replacement: "深夜",
      granularity: "token",
    }],
    warnings: [],
    status: "review",
    createdAt: at,
    proposalHash: "sha256:proposal",
  };
  const result: OperationExecutionResult = {
    kind: "patch_proposal",
    runId: proposal.operationRunId,
    providerId: "deepseek",
    model: "deepseek-v4-flash",
    responseId: "response-1",
    contextPacket: contextPacket(),
    proposal,
    usage: { inputTokens: 80, outputTokens: 20, totalTokens: 100 },
    finishReason: "stop",
    attempts: [{
      sequence: 1,
      startedAt: at,
      finishedAt: at,
      outcome: "succeeded",
      responseStarted: true,
      responseId: "response-1",
    }],
    history: history("review"),
  };
  const bundle = await buildSuccessfulPersistenceBundle({ intent, result, startedAt: at }, new Sha256Hasher());
  assert.equal(bundle.run.state, "review");
  assert.equal(bundle.artifact?.kind, "patch_proposal");
  assert.equal(bundle.artifact?.bindingHash, proposal.proposalHash);
  assert.equal(bundle.contextPacket?.payload.packetHash, "sha256:context");
  assert.equal(bundle.attempts?.[0]?.outcome, "succeeded");
  assert.equal(serializeOperationPersistenceBundle(bundle), serializeOperationPersistenceBundle(bundle));
  assert.equal(serializeOperationPersistenceBundle(bundle).includes("apiKey"), false);

  const endpointId = `endpoint-${"a".repeat(32)}`;
  const compatibleResult: OperationExecutionResult = {
    ...result,
    providerId: "openai_compatible",
  };
  const compatible = await buildSuccessfulPersistenceBundle({
    intent,
    result: compatibleResult,
    startedAt: at,
    providerConfigurationId: `provider-openai-compatible-${endpointId}`,
    providerEndpointId: endpointId,
  }, new Sha256Hasher());
  assert.equal(compatible.run.providerEndpointId, endpointId);
  await assert.rejects(
    buildSuccessfulPersistenceBundle({
      intent,
      result: compatibleResult,
      startedAt: at,
    }, new Sha256Hasher()),
    /providerEndpointId must not be empty/,
  );
  await assert.rejects(
    buildSuccessfulPersistenceBundle({
      intent,
      result: compatibleResult,
      startedAt: at,
      providerConfigurationId: "provider-forged",
      providerEndpointId: endpointId,
    }, new Sha256Hasher()),
    /providerConfigurationId does not match providerEndpointId/,
  );
});

test("maps failed execution metadata and retry safety without fabricating an artifact", () => {
  const intent = createIntent();
  const providerFailure = new ProviderError({
    kind: "server",
    providerId: "qwen",
    message: "Qwen request failed",
    retriable: true,
  });
  const error = new OperationExecutionError({
    code: "PROVIDER_FAILED",
    message: "Qwen request failed",
    runId: "run-failed-1" as OperationRunId,
    state: "failed",
    history: history("failed"),
    providerId: "qwen",
    model: "qwen-plus",
    contextPacket: contextPacket(),
    attempts: [{
      sequence: 1,
      startedAt: at,
      finishedAt: at,
      outcome: "failed",
      responseStarted: false,
      failure: { code: "PROVIDER_SERVER", kind: "server", retriable: true },
    }],
    rootCause: providerFailure,
  });
  const bundle = buildFailedPersistenceBundle({ intent, error, startedAt: at });
  assert.equal(bundle.run.state, "failed");
  assert.equal(bundle.run.failure?.retriable, true);
  assert.equal(bundle.run.failure?.code, "PROVIDER_FAILED");
  assert.equal(bundle.artifact, undefined);
  assert.equal(bundle.contextPacket?.id, "context-persist-1");
  assert.equal(bundle.attempts?.[0]?.failure?.kind, "server");

  const unresolved = buildFailedPersistenceBundle({
    intent,
    error: new OperationExecutionError({
      code: "INTERNAL_FAILURE",
      message: "Model resolution failed",
      runId: "run-failed-unresolved" as OperationRunId,
      state: "failed",
      history: history("failed"),
      providerId: "qwen",
      rootCause: new Error("resolution failed"),
    }),
    startedAt: at,
    requestedModel: "   ",
  });
  assert.equal(unresolved.run.model, "unresolved");
});

test("requires an explicit ID before persisting findings", async () => {
  const intent = createIntent({ type: "critique", output: { kind: "findings", maxTokens: 200 } });
  const result: OperationExecutionResult = {
    kind: "findings",
    runId: "run-findings-1" as OperationRunId,
    providerId: "minimax",
    model: "MiniMax-M3",
    responseId: "response-findings",
    contextPacket: contextPacket(),
    findings: [{ severity: "warning", message: "节奏过快" }],
    finishReason: "stop",
    attempts: [{
      sequence: 1,
      startedAt: at,
      finishedAt: at,
      outcome: "succeeded",
      responseStarted: true,
      responseId: "response-findings",
    }],
    history: history("review"),
  };
  await assert.rejects(
    buildSuccessfulPersistenceBundle({ intent, result, startedAt: at }, new Sha256Hasher()),
    /findingsArtifactId must not be empty/,
  );
  const bundle = await buildSuccessfulPersistenceBundle({
    intent,
    result,
    startedAt: at,
    findingsArtifactId: "findings-1",
  }, new Sha256Hasher());
  assert.equal(bundle.artifact?.kind, "findings");
  assert.match(bundle.artifact?.bindingHash ?? "", /^sha256:/);
});
