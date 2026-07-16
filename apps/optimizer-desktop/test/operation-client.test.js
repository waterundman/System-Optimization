import assert from "node:assert/strict";
import { createHash } from "node:crypto";
import test from "node:test";

import {
  credentialReference,
  defaultProviderSettings,
  hydrateDesktopReviewCandidate,
  matchesDesktopRetryTarget,
  providerConfiguration,
  providerRequiresCredential,
  planDesktopReviewDecisions,
  retryableOperationFailure,
  runDesktopOperation,
} from "../dist/operation-client.js";
import { ProviderError } from "../dist/runtime/packages/model-gateway/src/index.js";
import { OperationExecutionError } from "../dist/runtime/packages/operation-runner/src/index.js";

test("exposes only retriable failed operations for an explicit new run", () => {
  const providerFailure = new ProviderError({
    kind: "server",
    providerId: "deepseek",
    message: "Provider temporarily unavailable",
    code: "PROVIDER_SERVER",
    retriable: true,
  });
  const error = new OperationExecutionError({
    code: "PROVIDER_FAILED",
    message: providerFailure.message,
    runId: "run-retry-ui",
    state: "failed",
    history: [{ from: "draft", to: "failed", occurredAt: "2026-07-16T00:00:00Z" }],
    providerId: "deepseek",
    model: "deepseek-v4-flash",
    attempts: [
      {
        sequence: 1,
        startedAt: "2026-07-16T00:00:00Z",
        finishedAt: "2026-07-16T00:00:01Z",
        outcome: "failed",
        responseStarted: false,
        failure: { code: "PROVIDER_SERVER", kind: "server", retriable: true },
        retryDelayMs: 500,
      },
      {
        sequence: 2,
        startedAt: "2026-07-16T00:00:02Z",
        finishedAt: "2026-07-16T00:00:03Z",
        outcome: "failed",
        responseStarted: false,
        failure: { code: "PROVIDER_SERVER", kind: "server", retriable: true },
      },
    ],
    rootCause: providerFailure,
  });
  assert.deepEqual(retryableOperationFailure(error), {
    attempts: 2,
    code: "PROVIDER_SERVER",
    retryAfterMs: undefined,
  });
  assert.equal(retryableOperationFailure(new Error("not an operation failure")), null);
});

test("binds an explicit retry to the exact original block revision, hash and range", () => {
  const block = {
    id: "block-retry",
    revision: 4,
    contentHash: "sha256:retry-base",
    plainText: "需要重试的正文",
  };
  const target = {
    blockId: block.id,
    baseRevision: block.revision,
    baseHash: block.contentHash,
    from: 2,
    to: 6,
  };
  assert.equal(matchesDesktopRetryTarget(block, target), true);
  assert.equal(matchesDesktopRetryTarget({ ...block, revision: 5 }, target), false);
  assert.equal(matchesDesktopRetryTarget(block, { ...target, baseHash: "sha256:changed" }), false);
  assert.equal(matchesDesktopRetryTarget(block, { ...target, from: 2.5 }), false);
  assert.equal(matchesDesktopRetryTarget(block, { ...target, to: block.plainText.length + 1 }), false);
});

test("plans auditable batch review decisions without duplicating atomic groups", () => {
  const proposal = {
    id: "proposal-batch",
    proposalHash: `sha256:${"a".repeat(64)}`,
    hunks: [
      { id: "hunk-1", atomicGroup: "group-1" },
      { id: "hunk-2", atomicGroup: "group-1" },
      { id: "hunk-3" },
    ],
  };
  const session = {
    proposalId: proposal.id,
    proposalHash: proposal.proposalHash,
    revision: 0,
    status: "review",
    decisions: { "hunk-1": "pending", "hunk-2": "pending", "hunk-3": "pending" },
  };
  const accepted = planDesktopReviewDecisions(proposal, session, "accepted");
  assert.equal(accepted.steps.length, 2);
  assert.deepEqual(accepted.steps.map((step) => step.hunkId), ["hunk-1", "hunk-3"]);
  assert.equal(accepted.steps[0].previous.revision, 0);
  assert.equal(accepted.steps[1].previous.revision, 1);
  assert.equal(accepted.session.revision, 2);
  assert.equal(accepted.session.status, "ready");
  assert.deepEqual(new Set(Object.values(accepted.session.decisions)), new Set(["accepted"]));
  assert.throws(() => planDesktopReviewDecisions(proposal, session, "pending"), TypeError);
});

test("builds fixed host configurations for cloud and local providers", () => {
  const settings = defaultProviderSettings();
  assert.deepEqual(Object.keys(settings), ["deepseek", "qwen", "kimi", "minimax", "ollama"]);
  for (const providerId of Object.keys(settings)) {
    settings[providerId].enabled = true;
    const configuration = providerConfiguration(
      settings[providerId],
      "2026-07-15T00:00:00.000Z",
    );
    assert.equal(configuration.providerId, providerId);
    assert.equal(configuration.credentialRef, credentialReference(providerId));
    assert.equal("apiKey" in configuration, false);
  }
  assert.equal(providerRequiresCredential("deepseek"), true);
  assert.equal(providerRequiresCredential("ollama"), false);
  assert.equal(settings.ollama.defaultModel, "qwen3:8b");
});

test("rejects tampered unified host operation context before authorization", async () => {
  const providerSettings = defaultProviderSettings().deepseek;
  providerSettings.enabled = true;
  providerSettings.credentialExists = true;
  const block = {
    id: "block-tampered",
    documentId: "document-tampered",
    kind: "paragraph",
    orderKey: "a0",
    content: { type: "paragraph", content: [] },
    plainText: "目标正文",
    contentHash: `sha256:${"0".repeat(64)}`,
    revision: 0,
    locked: false,
  };
  const workspace = {
    schemaVersion: 1,
    projectId: "project-tampered",
    mainBranchId: "branch-tampered",
    headCommitId: "commit-tampered",
    revision: 0,
    documents: [{
      id: block.documentId,
      parentId: null,
      kind: "chapter",
      title: "正文",
      orderKey: "a0",
      revision: 0,
    }],
    blocks: [block],
  };
  const hostCalls = [];
  await assert.rejects(
    runDesktopOperation({
      invokeHost: async (command) => {
        hostCalls.push(command);
        if (command === "persist_operation_bundle") {
          return { schemaVersion: 1, state: "failed" };
        }
        throw new Error(`Unexpected host command: ${command}`);
      },
      createChannel: () => ({ onmessage: null }),
      providerSettings,
      workspace,
      block,
      from: 0,
      to: block.plainText.length,
      operationType: "polish",
      loadOperationContext: async () => [
        hostContextCandidate({
          id: "target-tampered",
          sourceRef: "block:block-tampered@r0#0-4",
          sourceCommitId: "commit-tampered",
          tier: "L0_TARGET",
          authority: "user_confirmed",
          renderMode: "verbatim",
          reasonCode: "USER_TARGET",
          content: "目标正文",
          mandatory: true,
          selectedByUser: true,
          relevance: 1,
          structuralProximity: 1,
        }),
        hostContextCandidate({
          id: "summary-tampered",
          sourceRef: "summary:project:project-tampered@commit-tampered",
          sourceCommitId: "commit-tampered",
          sourceHash: `sha256:${"f".repeat(64)}`,
          tier: "L3_KNOWLEDGE",
          authority: "source_derived",
          renderMode: "summary",
          reasonCode: "PROJECT_SUMMARY",
          content: "已被篡改的摘要",
        }),
      ],
    }),
  );
  assert.deepEqual(hostCalls, ["persist_operation_bundle"]);
  assert.equal(hostCalls.includes("authorize_model_request"), false);
  assert.equal(hostCalls.includes("execute_authorized_model_stream"), false);
});

test("runs Context Compiler to host stream to persisted patch proposal without plaintext secrets", async () => {
  const persisted = [];
  let confirmedContext = null;
  const providerSettings = defaultProviderSettings().deepseek;
  providerSettings.enabled = true;
  providerSettings.credentialExists = true;
  const block = {
    id: "block-1",
    documentId: "document-1",
    kind: "paragraph",
    orderKey: "a0",
    content: { type: "paragraph", content: [] },
    plainText: "",
    contentHash: `sha256:${"0".repeat(64)}`,
    revision: 0,
    locked: false,
  };
  const workspace = {
    schemaVersion: 1,
    projectId: "project-1",
    mainBranchId: "branch-1",
    headCommitId: "commit-1",
    revision: 0,
    documents: [{
      id: "document-1",
      parentId: null,
      kind: "chapter",
      title: "正文",
      orderKey: "a0",
      revision: 0,
    }],
    blocks: [block],
  };
  let authorizedRequest = null;
  const authorizedRequestIds = [];
  let modelExecutions = 0;
  const hostCalls = [];
  const invokeHost = async (command, args) => {
    hostCalls.push(command);
    if (command === "authorize_model_request") {
      assert.ok(confirmedContext, "context must be confirmed before the billable host request");
      assert.equal(args.input.binding.projectId, workspace.projectId);
      assert.equal(args.input.binding.baseCommitId, workspace.headCommitId);
      assert.equal(args.input.binding.operationIntentId, confirmedContext.operationIntentId);
      assert.equal(args.input.binding.contextPacketId, confirmedContext.id);
      assert.equal(args.input.binding.contextPacketHash, confirmedContext.packetHash);
      assert.equal(args.input.binding.providerLocality, "remote");
      assert.equal(args.input.binding.targetBlockId, block.id);
      assert.equal(args.input.binding.targetBlockRevision, block.revision);
      assert.equal(args.input.binding.targetBlockHash, block.contentHash);
      assert.equal(args.input.binding.targetFrom, 0);
      assert.equal(args.input.binding.targetTo, 0);
      assert.deepEqual(args.input.contextPacket, confirmedContext);
      authorizedRequest = args.input.request;
      authorizedRequestIds.push(authorizedRequest.requestId);
      return {
        schemaVersion: 1,
        authorizationId: `model-auth-test-${authorizedRequest.requestId}`,
        requestId: authorizedRequest.requestId,
        providerId: "deepseek",
        expiresAt: "2026-07-15T00:02:00Z",
      };
    }
    if (command === "execute_authorized_model_stream") {
      assert.ok(authorizedRequest, "a host authorization must precede execution");
      assert.equal(args.authorizationId, `model-auth-test-${authorizedRequest.requestId}`);
      modelExecutions += 1;
      if (modelExecutions === 1) {
        throw {
          code: "PROVIDER_RATE_LIMIT",
          message: "Provider temporarily rate limited the request",
          details: {
            status: 429,
            remoteCode: "rate_limit",
            remoteRequestId: "request-rate-limit-1",
            retryAfterMs: 800,
            retriable: true,
          },
        };
      }
      const output = JSON.stringify({
        schemaVersion: 1,
        kind: "replacement",
        replacementText: "你好，世界",
        summary: "插入开场句",
      });
      for (const event of [
        { type: "start", requestId: authorizedRequest.requestId, id: "response-1", providerId: "deepseek", model: "deepseek-v4-flash" },
        { type: "text_delta", text: output },
        { type: "finish", reason: "stop" },
      ]) args.onEvent.onmessage(event);
      return {
        schemaVersion: 1,
        requestId: authorizedRequest.requestId,
        responseId: "response-1",
        providerId: "deepseek",
        model: "deepseek-v4-flash",
        content: output,
        reasoningContent: "",
        finishReason: "stop",
      };
    }
    if (command === "persist_operation_bundle") {
      persisted.push(args.input);
      return { schemaVersion: 1, state: "review" };
    }
    throw new Error(`Unexpected host command: ${command}`);
  };

  const execution = await runDesktopOperation({
    invokeHost,
    createChannel: () => ({ onmessage: null }),
    providerSettings,
    workspace,
    block,
    from: 0,
    to: 0,
    operationType: "continue_scene",
    sleep: async (milliseconds) => { assert.equal(milliseconds, 800); },
    loadOperationContext: async (binding) => {
      hostCalls.push("get_operation_context");
      assert.equal(binding.baseCommitId, workspace.headCommitId);
      assert.equal(binding.targetBlockId, block.id);
      assert.equal(binding.targetBlockHash, block.contentHash);
      assert.equal(binding.from, 0);
      assert.equal(binding.to, 0);
      return [
        hostContextCandidate({
          id: "target-block-1",
          sourceRef: "block:block-1@r0#0-0",
          tier: "L0_TARGET",
          authority: "user_confirmed",
          renderMode: "verbatim",
          reasonCode: "USER_TARGET",
          content: "",
          mandatory: true,
          selectedByUser: true,
          relevance: 1,
          structuralProximity: 1,
        }),
        hostContextCandidate({
          id: "summary-project-1",
          sourceRef: "summary:project:project-1@commit-1",
          tier: "L3_KNOWLEDGE",
          authority: "source_derived",
          renderMode: "summary",
          reasonCode: "PROJECT_SUMMARY",
          content: "项目摘要：这是宿主生成并绑定到当前 Commit 的摘要。",
        }),
        hostContextCandidate({
          id: "fact-1",
          sourceRef: "knowledge:fact:fact-1@r0",
          tier: "L3_KNOWLEDGE",
          authority: "user_confirmed",
          renderMode: "constraint",
          content: "事实【主角视觉】：主角左眼失明",
          sensitivity: "local_sensitive",
          reasonCode: "CANONICAL_FACT",
          selectedByUser: true,
        }),
        hostContextCandidate({
          id: "constraint-1",
          sourceRef: "knowledge:constraint:constraint-1@r0",
          tier: "L3_KNOWLEDGE",
          authority: "user_confirmed",
          renderMode: "constraint",
          content: "硬约束【禁止剧透】：本章不得揭示凶手身份",
          sensitivity: "never_send",
          reasonCode: "PROJECT_HARD_CONSTRAINT",
          selectedByUser: true,
        }),
        hostContextCandidate({
          id: "style-canonical",
          sourceRef: "style:style-canonical@r0",
          tier: "L4_STYLE_GLOBAL",
          authority: "user_confirmed",
          renderMode: "verbatim",
          content: "雨很轻。灯还亮着。",
          reasonCode: "PINNED_STYLE_SAMPLE",
          selectedByUser: true,
        }),
        hostContextCandidate({
          id: "style-archived",
          sourceRef: "style:style-archived@r0",
          tier: "L4_STYLE_GLOBAL",
          status: "archived",
          authority: "user_confirmed",
          renderMode: "verbatim",
          content: "不应被重新召回。",
          reasonCode: "PINNED_STYLE_SAMPLE",
        }),
        hostContextCandidate({
          id: "style-local-only",
          sourceRef: "style:style-local-only@r0",
          tier: "L4_STYLE_GLOBAL",
          authority: "user_confirmed",
          renderMode: "verbatim",
          content: "远程调用不可发送。",
          sensitivity: "never_send",
          reasonCode: "PINNED_STYLE_SAMPLE",
          selectedByUser: true,
        }),
      ];
    },
    confirmContext: async (packet) => {
      confirmedContext = packet;
    },
  });

  assert.equal(confirmedContext.operationIntentId, execution.intent.id);
  assert.ok(confirmedContext.items.length >= 1);
  assert.ok(confirmedContext.items.some((item) => item.sourceRef === "style:style-canonical@r0"));
  assert.ok(confirmedContext.items.some((item) => item.sourceRef === "summary:project:project-1@commit-1"));
  assert.ok(confirmedContext.items.some((item) => item.sourceRef === "knowledge:fact:fact-1@r0"));
  assert.equal(
    confirmedContext.exclusions.find((item) => item.sourceRef === "knowledge:constraint:constraint-1@r0")?.reason,
    "POLICY_DENIED",
  );
  assert.deepEqual(
    Object.fromEntries(confirmedContext.exclusions
      .filter((item) => item.sourceRef.startsWith("style:"))
      .map((item) => [item.sourceRef, item.reason])),
    {
      "style:style-archived@r0": "INELIGIBLE_STATUS",
      "style:style-local-only@r0": "POLICY_DENIED",
    },
  );
  assert.equal(execution.result.kind, "patch_proposal");
  assert.equal(execution.result.proposal.hunks.length, 1);
  assert.equal(execution.result.proposal.hunks[0].replacement, "你好，世界");
  const persistedSession = {
    proposalId: execution.result.proposal.id,
    proposalHash: execution.result.proposal.proposalHash,
    revision: 1,
    status: "ready",
    decisions: Object.fromEntries(
      execution.result.proposal.hunks.map((hunk) => [hunk.id, "accepted"]),
    ),
  };
  const hydrated = await hydrateDesktopReviewCandidate({
    schemaVersion: 1,
    summary: { candidateBranch: null },
    proposal: execution.result.proposal,
    session: persistedSession,
  });
  assert.deepEqual(hydrated.session, persistedSession);
  await assert.rejects(
    hydrateDesktopReviewCandidate({
      schemaVersion: 1,
      summary: { candidateBranch: null },
      proposal: execution.result.proposal,
      session: {
        ...persistedSession,
        proposalHash: "sha256:" + "f".repeat(64),
      },
    }),
    TypeError,
  );
  await assert.rejects(
    hydrateDesktopReviewCandidate({
      schemaVersion: 1,
      summary: { candidateBranch: null },
      proposal: execution.result.proposal,
      session: { ...persistedSession, status: "review" },
    }),
    TypeError,
  );
  assert.deepEqual(hostCalls.slice(0, 5), [
    "get_operation_context",
    "authorize_model_request",
    "execute_authorized_model_stream",
    "authorize_model_request",
    "execute_authorized_model_stream",
  ]);
  assert.equal(persisted.length, 1);
  const bundle = JSON.parse(persisted[0]);
  assert.equal(bundle.run.state, "review");
  assert.equal(bundle.artifact.kind, "patch_proposal");
  assert.equal(bundle.attempts.length, 2);
  assert.equal(bundle.attempts[0].outcome, "failed");
  assert.equal(bundle.attempts[0].failure.kind, "rate_limit");
  assert.equal(bundle.attempts[0].retryDelayMs, 800);
  assert.equal(bundle.attempts[1].outcome, "succeeded");
  assert.equal(authorizedRequestIds.length, 2);
  assert.equal(new Set(authorizedRequestIds).size, 2);
  assert.equal(persisted[0].includes("apiKey"), false);
  assert.equal(persisted[0].includes("secret-never"), false);
});

function hostContextCandidate({
  id,
  sourceRef,
  sourceCommitId = "commit-1",
  sourceHash,
  tier,
  status = "canonical",
  authority,
  sensitivity = "local_sensitive",
  renderMode,
  reasonCode,
  content,
  mandatory = false,
  selectedByUser = false,
  relevance = 0.8,
  structuralProximity = 0.5,
}) {
  return {
    id,
    sourceRef,
    sourceHash: sourceHash ?? `sha256:${createHash("sha256").update(content).digest("hex")}`,
    sourceCommitId,
    tier,
    status,
    authority,
    sensitivity,
    renderMode,
    reasonCodes: [reasonCode],
    content,
    mandatory,
    selectedByUser,
    signals: { relevance, structuralProximity, freshness: 1, risk: 0 },
  };
}
