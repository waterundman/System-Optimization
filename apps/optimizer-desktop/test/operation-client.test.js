import assert from "node:assert/strict";
import { createHash } from "node:crypto";
import test from "node:test";

import {
  credentialReference,
  defaultProviderSettings,
  providerConfiguration,
  providerRequiresCredential,
  runDesktopOperation,
} from "../dist/operation-client.js";

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

test("rejects tampered host summary context before authorization", async () => {
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
      projectTitle: "测试项目",
      block,
      from: 0,
      to: block.plainText.length,
      operationType: "polish",
      loadSummaryContext: async () => [{
        id: "summary-tampered",
        sourceRef: "summary:project:project-tampered@commit-tampered",
        sourceHash: `sha256:${"f".repeat(64)}`,
        tier: "L3_KNOWLEDGE",
        status: "canonical",
        authority: "source_derived",
        sensitivity: "local_sensitive",
        renderMode: "summary",
        reasonCodes: ["PROJECT_SUMMARY"],
        content: "已被篡改的摘要",
      }],
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
      authorizedRequest = args.input.request;
      return {
        schemaVersion: 1,
        authorizationId: "model-auth-test",
        requestId: authorizedRequest.requestId,
        providerId: "deepseek",
        expiresAt: "2026-07-15T00:02:00Z",
      };
    }
    if (command === "execute_authorized_model_stream") {
      assert.ok(authorizedRequest, "a host authorization must precede execution");
      assert.equal(args.authorizationId, "model-auth-test");
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
    projectTitle: "测试项目",
    block,
    from: 0,
    to: 0,
    operationType: "continue_scene",
    styleSamples: [
      {
        id: "style-canonical",
        title: "短句",
        content: "雨很轻。灯还亮着。",
        status: "canonical",
        sensitivity: "local_sensitive",
      },
      {
        id: "style-archived",
        title: "废弃样本",
        content: "不应被重新召回。",
        status: "archived",
        sensitivity: "local_sensitive",
      },
      {
        id: "style-local-only",
        title: "仅本地",
        content: "远程调用不可发送。",
        status: "canonical",
        sensitivity: "never_send",
      },
    ],
    loadSummaryContext: async (binding) => {
      hostCalls.push("get_summary_context");
      assert.equal(binding.baseCommitId, workspace.headCommitId);
      assert.equal(binding.targetBlockId, block.id);
      const content = "项目摘要：这是宿主生成并绑定到当前 Commit 的摘要。";
      return [{
        id: "summary-project-1",
        sourceRef: "summary:project:project-1@commit-1",
        sourceHash: `sha256:${createHash("sha256").update(content).digest("hex")}`,
        sourceCommitId: "commit-1",
        tier: "L3_KNOWLEDGE",
        status: "canonical",
        authority: "source_derived",
        sensitivity: "local_sensitive",
        renderMode: "summary",
        reasonCodes: ["PROJECT_SUMMARY"],
        content,
        revision: 0,
        generatedAt: "2026-07-15T00:00:00Z",
      }];
    },
    confirmContext: async (packet) => {
      confirmedContext = packet;
    },
  });

  assert.equal(confirmedContext.operationIntentId, execution.intent.id);
  assert.ok(confirmedContext.items.length >= 1);
  assert.ok(confirmedContext.items.some((item) => item.sourceRef === "style:style-canonical"));
  assert.ok(confirmedContext.items.some((item) => item.sourceRef === "summary:project:project-1@commit-1"));
  assert.deepEqual(
    Object.fromEntries(confirmedContext.exclusions
      .filter((item) => item.sourceRef.startsWith("style:"))
      .map((item) => [item.sourceRef, item.reason])),
    {
      "style:style-archived": "INELIGIBLE_STATUS",
      "style:style-local-only": "POLICY_DENIED",
    },
  );
  assert.equal(execution.result.kind, "patch_proposal");
  assert.equal(execution.result.proposal.hunks.length, 1);
  assert.equal(execution.result.proposal.hunks[0].replacement, "你好，世界");
  assert.deepEqual(hostCalls.slice(0, 3), [
    "get_summary_context",
    "authorize_model_request",
    "execute_authorized_model_stream",
  ]);
  assert.equal(persisted.length, 1);
  const bundle = JSON.parse(persisted[0]);
  assert.equal(bundle.run.state, "review");
  assert.equal(bundle.artifact.kind, "patch_proposal");
  assert.equal(persisted[0].includes("apiKey"), false);
  assert.equal(persisted[0].includes("secret-never"), false);
});
