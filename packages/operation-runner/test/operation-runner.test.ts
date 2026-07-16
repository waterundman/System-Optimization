import assert from "node:assert/strict";
import test from "node:test";
import type {
  BlockId,
  ContextPacket,
  DocumentId,
  ModelProviderId,
  OperationRunId,
  PatchProposalId,
} from "../../protocol/src/index.ts";
import type { EditorBlockSnapshot } from "../../editor-bridge/src/index.ts";
import { ContextCompiler } from "../../kernel/src/index.ts";
import {
  deepSeekProfile,
  ProviderError,
  type ModelGatewayOptions,
  type ModelProvider,
  type ModelRequest,
  type ModelStreamEvent,
  type ProviderProfile,
} from "../../model-gateway/src/index.ts";
import {
  ApproxTokenEstimator,
  FixedClock,
  SequentialIds,
  Sha256Hasher,
  createIntent,
} from "../../test-kit/src/index.ts";
import {
  OperationExecutionError,
  OperationRunner,
  parseModelOutput,
  type OperationProgressEvent,
} from "../src/index.ts";

const at = "2026-07-15T00:00:00.000Z";

class RunnerIds {
  private value = 0;
  nextRunId(): OperationRunId {
    this.value += 1;
    return `run-test-${this.value}` as OperationRunId;
  }
  nextProposalId(): PatchProposalId {
    this.value += 1;
    return `proposal-test-${this.value}` as PatchProposalId;
  }
}

class FakeProvider implements ModelProvider {
  readonly requests: ModelRequest[] = [];
  readonly profile: ProviderProfile;
  private readonly events: readonly ModelStreamEvent[];
  private readonly failures: ProviderError[];
  private readonly failureAfterStart?: ProviderError;
  streamCalls = 0;

  constructor(
    profile: ProviderProfile,
    events: readonly ModelStreamEvent[],
    failures: readonly ProviderError[] = [],
    failureAfterStart?: ProviderError,
  ) {
    this.profile = profile;
    this.events = events;
    this.failures = [...failures];
    this.failureAfterStart = failureAfterStart;
  }

  async complete(): Promise<never> {
    throw new Error("complete() is not used by OperationRunner");
  }

  async *stream(
    request: ModelRequest,
    _options?: ModelGatewayOptions,
  ): AsyncGenerator<ModelStreamEvent> {
    this.streamCalls += 1;
    this.requests.push(request);
    const failure = this.failures.shift();
    if (failure) throw failure;
    for (const event of this.events) {
      yield event;
      if (event.type === "start" && this.failureAfterStart) throw this.failureAfterStart;
    }
  }
}

function fixture(options: {
  readonly modelOutput?: string;
  readonly events?: readonly ModelStreamEvent[];
  readonly profile?: ProviderProfile;
  readonly extraCandidates?: readonly ReturnType<typeof candidate>[];
  readonly mutatePacket?: (packet: ContextPacket) => ContextPacket;
  readonly failures?: readonly ProviderError[];
  readonly sleep?: (milliseconds: number, signal?: AbortSignal) => Promise<void>;
  readonly failureAfterStart?: ProviderError;
} = {}) {
  const text = "夜雨敲打窗棂。";
  const blockId = "block-1" as BlockId;
  const documentId = "document-1" as DocumentId;
  const block: EditorBlockSnapshot = {
    id: blockId,
    documentId,
    kind: "paragraph",
    orderKey: "a",
    content: { type: "paragraph" },
    plainText: text,
    contentHash: "sha256:base",
    revision: 3,
    locked: false,
  };
  const intent = createIntent({
    target: {
      documentId,
      blockId,
      baseRevision: block.revision,
      baseHash: block.contentHash,
      from: { blockId, offset: 0 },
      to: { blockId, offset: text.length },
    },
  });
  const contextCompiler = new ContextCompiler({
    contextSources: [{
      id: "target",
      async collect() {
        return [candidate(text), ...(options.extraCandidates ?? [])];
      },
    }],
    tokenizer: new ApproxTokenEstimator(),
    hasher: new Sha256Hasher(),
    clock: new FixedClock(at),
    ids: new SequentialIds(),
  });
  const modelOutput = options.modelOutput ?? JSON.stringify({
    schemaVersion: 1,
    kind: "replacement",
    replacementText: "夜雨轻轻敲打窗棂。",
    summary: "降低突兀感",
  });
  const events = options.events ?? [
    { type: "start", id: "response-1", providerId: "deepseek", model: "deepseek-v4-flash" },
    { type: "text_delta", text: modelOutput.slice(0, 20) },
    { type: "text_delta", text: modelOutput.slice(20) },
    { type: "usage", usage: { inputTokens: 80, outputTokens: 20, totalTokens: 100 } },
    { type: "finish", reason: "stop" },
  ];
  const provider = new FakeProvider(
    options.profile ?? deepSeekProfile,
    events,
    options.failures,
    options.failureAfterStart,
  );
  const compilationPort = options.mutatePacket
    ? {
        async compile(request: Parameters<ContextCompiler["compile"]>[0]) {
          return options.mutatePacket?.(await contextCompiler.compile(request))
            ?? await contextCompiler.compile(request);
        },
      }
    : contextCompiler;
  const runner = new OperationRunner({
    contextCompiler: compilationPort,
    providers: {
      provider(providerId: ModelProviderId) {
        assert.equal(providerId, provider.profile.id);
        return provider;
      },
    },
    hasher: new Sha256Hasher(),
    clock: new FixedClock(at),
    ids: new RunnerIds(),
    ...(options.sleep !== undefined ? { sleep: options.sleep } : {}),
  });
  return { runner, provider, block, intent, text };
}

function candidate(content: string, overrides: Record<string, unknown> = {}) {
  return {
    id: "candidate-target",
    sourceRef: "document-1#block-1:0-8",
    sourceHash: "sha256:target",
    tier: "L0_TARGET" as const,
    status: "canonical" as const,
    authority: "source_derived" as const,
    sensitivity: "local" as const,
    renderMode: "verbatim" as const,
    reasonCodes: ["CURRENT_SELECTION"],
    content,
    mandatory: true,
    ...overrides,
  };
}

test("executes ContextPacket to provider stream to PatchProposal end to end", async () => {
  const { runner, provider, block, intent } = fixture();
  const progress: OperationProgressEvent[] = [];
  const result = await runner.execute({
    intent,
    providerId: "deepseek",
    block,
    modelLimit: 8_000,
    reservedOverhead: 200,
    onProgress: (event) => progress.push(event),
  });

  assert.equal(result.kind, "patch_proposal");
  if (result.kind !== "patch_proposal") return;
  assert.equal(result.proposal.status, "review");
  assert.equal(result.proposal.operationRunId, result.runId);
  assert.equal(result.proposal.hunks.length > 0, true);
  assert.equal(result.usage?.totalTokens, 100);
  assert.deepEqual(result.history.map((entry) => entry.to), [
    "compiling", "preflight", "queued", "streaming", "validating", "review",
  ]);
  assert.equal(provider.requests[0]?.responseFormat, "json_object");
  assert.equal(provider.requests[0]?.maxOutputTokens, intent.output.maxTokens);
  assert.equal(progress.some((event) => event.type === "model_text_delta"), true);
  assert.deepEqual(result.attempts, [{
    sequence: 1,
    startedAt: at,
    finishedAt: at,
    outcome: "succeeded",
    responseStarted: true,
    responseId: "response-1",
  }]);
});

test("retries one pre-response retriable failure and audits both attempts", async () => {
  const retryable = new ProviderError({
    kind: "rate_limit",
    providerId: "deepseek",
    message: "Temporarily rate limited",
    status: 429,
    code: "rate_limit",
    requestId: "request-rate-limit",
    retryAfterMs: 800,
    retriable: true,
  });
  const delays: number[] = [];
  const progress: OperationProgressEvent[] = [];
  const { runner, provider, block, intent } = fixture({
    failures: [retryable],
    sleep: async (milliseconds) => { delays.push(milliseconds); },
  });
  const result = await runner.execute({
    intent,
    providerId: "deepseek",
    block,
    modelLimit: 8_000,
    retryPolicy: { maxAttempts: 2, baseDelayMs: 500, maxDelayMs: 5_000 },
    onProgress: (event) => progress.push(event),
  });

  assert.equal(provider.streamCalls, 2);
  assert.deepEqual(delays, [800]);
  assert.equal(result.attempts.length, 2);
  assert.deepEqual(result.attempts[0], {
    sequence: 1,
    startedAt: at,
    finishedAt: at,
    outcome: "failed",
    responseStarted: false,
    failure: {
      code: "rate_limit",
      kind: "rate_limit",
      status: 429,
      requestId: "request-rate-limit",
      retryAfterMs: 800,
      retriable: true,
    },
    retryDelayMs: 800,
  });
  assert.equal(result.attempts[1]?.outcome, "succeeded");
  assert.deepEqual(
    progress.filter((event) => event.type === "model_retry"),
    [{
      type: "model_retry",
      runId: result.runId,
      failedAttempt: 1,
      nextAttempt: 2,
      delayMs: 800,
      failureCode: "rate_limit",
    }],
  );
});

test("does not retry after a provider response has started", async () => {
  const failure = new ProviderError({
    kind: "network",
    providerId: "deepseek",
    message: "Connection reset after response headers",
    code: "PROVIDER_NETWORK",
    retriable: true,
  });
  const base = fixture({ failureAfterStart: failure });
  await assert.rejects(
    base.runner.execute({
      intent: base.intent,
      providerId: "deepseek",
      block: base.block,
      modelLimit: 8_000,
      retryPolicy: { maxAttempts: 3, baseDelayMs: 0, maxDelayMs: 1_000 },
    }),
    (error: unknown) => {
      assert.equal(error instanceof OperationExecutionError, true);
      if (!(error instanceof OperationExecutionError)) return false;
      assert.equal(error.attempts.length, 1);
      assert.equal(error.attempts[0]?.responseStarted, true);
      assert.equal(error.attempts[0]?.retryDelayMs, undefined);
      return true;
    },
  );
  assert.equal(base.provider.streamCalls, 1);
});

test("does not retry beyond the configured delay cap and bounds audit metadata", async () => {
  const failure = new ProviderError({
    kind: "rate_limit",
    providerId: "deepseek",
    message: "Retry later",
    code: `rate-limit-${"x".repeat(300)}`,
    requestId: `request-${"y".repeat(600)}`,
    retryAfterMs: 2_000,
    retriable: true,
  });
  const base = fixture({ failures: [failure] });
  await assert.rejects(
    base.runner.execute({
      intent: base.intent,
      providerId: "deepseek",
      block: base.block,
      modelLimit: 8_000,
      retryPolicy: { maxAttempts: 3, baseDelayMs: 500, maxDelayMs: 1_000 },
    }),
    (error: unknown) => {
      assert.equal(error instanceof OperationExecutionError, true);
      if (!(error instanceof OperationExecutionError)) return false;
      assert.equal(error.attempts.length, 1);
      assert.equal(error.attempts[0]?.failure?.code.length, 200);
      assert.equal(error.attempts[0]?.failure?.requestId?.length, 512);
      assert.equal(error.attempts[0]?.retryDelayMs, undefined);
      return true;
    },
  );
  assert.equal(base.provider.streamCalls, 1);
});

test("cancels during retry backoff before issuing another provider request", async () => {
  const controller = new AbortController();
  const failure = new ProviderError({
    kind: "network",
    providerId: "deepseek",
    message: "Temporary network failure",
    retriable: true,
  });
  const base = fixture({ failures: [failure] });
  await assert.rejects(
    base.runner.execute({
      intent: base.intent,
      providerId: "deepseek",
      block: base.block,
      modelLimit: 8_000,
      retryPolicy: { maxAttempts: 2, baseDelayMs: 500, maxDelayMs: 1_000 },
      signal: controller.signal,
      onProgress: (event) => {
        if (event.type === "model_retry") controller.abort();
      },
    }),
    (error: unknown) => {
      assert.equal(error instanceof OperationExecutionError, true);
      if (!(error instanceof OperationExecutionError)) return false;
      assert.equal(error.code, "OPERATION_CANCELLED");
      assert.equal(error.state, "cancelled");
      assert.equal(error.attempts.length, 1);
      return true;
    },
  );
  assert.equal(base.provider.streamCalls, 1);
});

test("derives remote locality from the provider and never renders never-send context", async () => {
  const secret = "NEVER_SEND_SECRET";
  const { runner, provider, block, intent } = fixture({
    extraCandidates: [candidate(secret, {
      id: "private",
      sourceRef: "private://note",
      sourceHash: "sha256:private",
      tier: "L3_KNOWLEDGE",
      sensitivity: "never_send",
      mandatory: false,
    })],
  });
  const result = await runner.execute({
    intent,
    providerId: "deepseek",
    block,
    modelLimit: 8_000,
  });
  assert.equal(result.contextPacket.exclusions.some((item) => item.reason === "POLICY_DENIED"), true);
  assert.equal(provider.requests[0]?.messages.some((message) => message.content.includes(secret)), false);
});

test("fails stale targets during preflight before a billable provider call", async () => {
  const { runner, provider, block, intent } = fixture();
  await assert.rejects(
    runner.execute({
      intent,
      providerId: "deepseek",
      block: { ...block, revision: block.revision + 1 },
      modelLimit: 8_000,
    }),
    (error: unknown) => {
      assert.equal(error instanceof OperationExecutionError, true);
      if (!(error instanceof OperationExecutionError)) return false;
      assert.equal(error.code, "TARGET_STALE");
      assert.equal(error.state, "failed");
      assert.equal(error.contextPacket?.id.startsWith("ctx_test_"), true);
      assert.equal(error.providerId, "deepseek");
      assert.equal(error.model, "deepseek-v4-flash");
      assert.equal(error.history.at(-1)?.to, "failed");
      return true;
    },
  );
  assert.equal(provider.streamCalls, 0);
});

test("detects a tampered ContextPacket before any provider call", async () => {
  const base = fixture({
    mutatePacket(packet) {
      return {
        ...packet,
        items: packet.items.map((item, index) => index === 0
          ? { ...item, content: `${item.content}篡改` }
          : item),
      };
    },
  });
  await assert.rejects(
    base.runner.execute({
      intent: base.intent,
      providerId: "deepseek",
      block: base.block,
      modelLimit: 8_000,
    }),
    (error: unknown) => error instanceof OperationExecutionError
      && error.code === "CONTEXT_INVALID",
  );
  assert.equal(base.provider.streamCalls, 0);
});

test("cancels a running stream even when a provider ignores its AbortSignal", async () => {
  const controller = new AbortController();
  const { runner, block, intent } = fixture();
  await assert.rejects(
    runner.execute({
      intent,
      providerId: "deepseek",
      block,
      modelLimit: 8_000,
      signal: controller.signal,
      onProgress(event) {
        if (event.type === "model_text_delta") controller.abort("user cancelled");
      },
    }),
    (error: unknown) => {
      assert.equal(error instanceof OperationExecutionError, true);
      if (!(error instanceof OperationExecutionError)) return false;
      assert.equal(error.code, "OPERATION_CANCELLED");
      assert.equal(error.state, "cancelled");
      assert.equal(error.history.at(-1)?.to, "cancelled");
      return true;
    },
  );
});

test("rejects fenced JSON and truncated output instead of producing a patch", async () => {
  const fenced = fixture({ modelOutput: "```json\n{}\n```" });
  await assert.rejects(
    fenced.runner.execute({
      intent: fenced.intent,
      providerId: "deepseek",
      block: fenced.block,
      modelLimit: 8_000,
    }),
    (error: unknown) => error instanceof OperationExecutionError
      && error.code === "MODEL_OUTPUT_INVALID",
  );

  const truncated = fixture({
    events: [
      { type: "start", id: "response-2", providerId: "deepseek", model: "deepseek-v4-flash" },
      { type: "text_delta", text: "{\"schemaVersion\":1" },
      { type: "finish", reason: "length" },
    ],
  });
  await assert.rejects(
    truncated.runner.execute({
      intent: truncated.intent,
      providerId: "deepseek",
      block: truncated.block,
      modelLimit: 8_000,
    }),
    (error: unknown) => error instanceof OperationExecutionError
      && error.code === "MODEL_OUTPUT_TRUNCATED",
  );
});

test("returns validated critique findings without compiling an editor patch", async () => {
  const output = JSON.stringify({
    schemaVersion: 1,
    kind: "findings",
    findings: [{ severity: "warning", message: "意象出现得过于密集", sourceRef: "document-1#block-1" }],
    summary: "节奏检查",
  });
  const base = fixture({ modelOutput: output });
  const intent = createIntent({
    ...base.intent,
    type: "critique",
    output: { kind: "findings", maxTokens: 200 },
  });
  const result = await base.runner.execute({
    intent,
    providerId: "deepseek",
    block: { ...base.block, locked: true },
    modelLimit: 8_000,
  });
  assert.equal(result.kind, "findings");
  if (result.kind === "findings") {
    assert.equal(result.findings[0]?.severity, "warning");
    assert.equal(result.summary, "节奏检查");
  }
});

test("strict output parser rejects unknown fields and wrong artifact kinds", () => {
  assert.throws(
    () => parseModelOutput(JSON.stringify({
      schemaVersion: 1,
      kind: "replacement",
      replacementText: "text",
      command: "overwrite project",
    }), "patch_proposal"),
    /Unknown model output field/,
  );
  assert.throws(
    () => parseModelOutput(JSON.stringify({
      schemaVersion: 1,
      kind: "findings",
      findings: [],
    }), "patch_proposal"),
    /Unknown model output field|Expected a replacement output/,
  );
});
