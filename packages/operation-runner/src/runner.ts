import type {
  ContextPacket,
  OperationRunId,
  OperationState,
} from "../../protocol/src/index.ts";
import {
  DomainError,
  OperationLifecycle,
  canTransition,
  coreOperationProfiles,
  stableStringify,
} from "../../kernel/src/index.ts";
import { PatchEngineError, compilePatchProposal } from "../../patch-engine/src/index.ts";
import {
  ProviderError,
  type FinishReason,
  type ModelStreamEvent,
  type ModelUsage,
} from "../../model-gateway/src/index.ts";
import {
  OperationExecutionError,
  OperationStageError,
  type OperationExecutionErrorCode,
} from "./errors.ts";
import { parseModelOutput } from "./model-output.ts";
import { renderOperationMessages } from "./prompt.ts";
import type {
  ExecuteOperationRequest,
  OperationAttempt,
  OperationAttemptFailure,
  OperationExecutionResult,
  OperationProgressEvent,
  OperationRunnerPorts,
} from "./types.ts";

const defaultReservedOverhead = 800;
const defaultMaxResponseBytes = 10 * 1024 * 1024;
const defaultRetryPolicy = Object.freeze({
  maxAttempts: 2,
  baseDelayMs: 500,
  maxDelayMs: 5_000,
});

export class OperationRunner {
  private readonly ports: OperationRunnerPorts;

  constructor(ports: OperationRunnerPorts) {
    this.ports = ports;
  }

  async execute(request: ExecuteOperationRequest): Promise<OperationExecutionResult> {
    const runId = this.ports.ids.nextRunId();
    const lifecycle = new OperationLifecycle();
    let compiledPacket: ContextPacket | undefined;
    let resolvedModel: string | undefined;
    const attempts: OperationAttempt[] = [];
    const move = (to: OperationState, reason?: string): void => {
      const transition = lifecycle.transition(to, this.ports.clock.now(), reason);
      emit(request.onProgress, { type: "lifecycle", runId, transition });
    };

    try {
      throwIfCancelled(request.signal);
      move("compiling");
      const provider = this.ports.providers.provider(request.providerId);
      resolvedModel = request.model?.trim() || provider.profile.defaultModel;
      const packet = await this.ports.contextCompiler.compile({
        intent: request.intent,
        profile: coreOperationProfiles[request.intent.type],
        providerLocality: provider.profile.locality,
        modelLimit: request.modelLimit,
        reservedOverhead: request.reservedOverhead ?? defaultReservedOverhead,
      });
      compiledPacket = packet;

      throwIfCancelled(request.signal);
      move("preflight");
      assertResponseLimit(request.maxResponseBytes);
      assertFreshTarget(request);
      await assertTargetContext(packet, request, this.ports.hasher);

      move("queued");
      throwIfCancelled(request.signal);
      move("streaming");
      const collected = await collectModelStreamWithRetry({
        provider,
        request,
        packet,
        runId,
        attempts,
        clock: this.ports.clock,
        sleep: this.ports.sleep ?? abortableSleep,
      });

      throwIfCancelled(request.signal);
      move("validating");
      assertUsableFinishReason(collected.finishReason);
      const output = parseModelOutput(collected.text, request.intent.output.kind);

      if (output.kind === "findings") {
        move("review");
        return {
          kind: "findings",
          runId,
          providerId: request.providerId,
          model: collected.model,
          responseId: collected.responseId,
          contextPacket: packet,
          findings: output.findings,
          ...(output.summary !== undefined ? { summary: output.summary } : {}),
          ...(collected.usage !== undefined ? { usage: collected.usage } : {}),
          finishReason: collected.finishReason,
          attempts: [...attempts],
          history: lifecycle.history,
        };
      }

      const proposal = await compilePatchProposal({
        id: this.ports.ids.nextProposalId(),
        operationRunId: runId,
        baseCommitId: request.intent.baseCommitId,
        target: request.intent.target,
        block: request.block,
        replacementText: output.replacementText,
        createdAt: this.ports.clock.now(),
        summary: output.summary,
        atomic: request.atomicPatch,
      }, this.ports.hasher);
      move("review");
      return {
        kind: "patch_proposal",
        runId,
        providerId: request.providerId,
        model: collected.model,
        responseId: collected.responseId,
        contextPacket: packet,
        proposal,
        ...(collected.usage !== undefined ? { usage: collected.usage } : {}),
        finishReason: collected.finishReason,
        attempts: [...attempts],
        history: lifecycle.history,
      };
    } catch (error) {
      const cancelled = request.signal?.aborted === true
        || (error instanceof ProviderError && error.kind === "cancelled")
        || (error instanceof OperationStageError && error.code === "OPERATION_CANCELLED");
      const terminal: "cancelled" | "failed" = cancelled ? "cancelled" : "failed";
      if (canTransition(lifecycle.state, terminal)) move(terminal, safeReason(error));
      const code = cancelled ? "OPERATION_CANCELLED" : classifyError(error);
      throw new OperationExecutionError({
        code,
        message: safeMessage(error, cancelled),
        runId,
        state: terminal,
        history: lifecycle.history,
        providerId: request.providerId,
        model: resolvedModel,
        contextPacket: compiledPacket,
        attempts,
        rootCause: error,
      });
    }
  }
}

interface CollectedStream {
  readonly responseId: string;
  readonly model: string;
  readonly text: string;
  readonly usage?: ModelUsage;
  readonly finishReason: FinishReason;
}

class ModelAttemptError extends Error {
  readonly rootCause: unknown;
  readonly responseStarted: boolean;
  readonly responseId?: string;

  constructor(rootCause: unknown, responseId?: string) {
    super("Model attempt failed");
    this.name = "ModelAttemptError";
    this.rootCause = rootCause;
    this.responseStarted = responseId !== undefined;
    this.responseId = responseId;
  }
}

async function collectModelStreamWithRetry(input: {
  readonly provider: ReturnType<OperationRunnerPorts["providers"]["provider"]>;
  readonly request: ExecuteOperationRequest;
  readonly packet: ContextPacket;
  readonly runId: OperationRunId;
  readonly attempts: OperationAttempt[];
  readonly clock: OperationRunnerPorts["clock"];
  readonly sleep: NonNullable<OperationRunnerPorts["sleep"]>;
}): Promise<CollectedStream> {
  const policy = normalizeRetryPolicy(input.request.retryPolicy);
  for (let sequence = 1; sequence <= policy.maxAttempts; sequence += 1) {
    throwIfCancelled(input.request.signal);
    const startedAt = input.clock.now();
    try {
      const collected = await collectModelStream(input);
      input.attempts.push({
        sequence,
        startedAt,
        finishedAt: input.clock.now(),
        outcome: "succeeded",
        responseStarted: true,
        responseId: collected.responseId,
      });
      return collected;
    } catch (caught) {
      const attemptError = caught instanceof ModelAttemptError
        ? caught
        : new ModelAttemptError(caught);
      const failure = operationAttemptFailure(attemptError.rootCause);
      const retryDelayMs = retryDelayFor({
        error: attemptError.rootCause,
        responseStarted: attemptError.responseStarted,
        sequence,
        policy,
      });
      input.attempts.push({
        sequence,
        startedAt,
        finishedAt: input.clock.now(),
        outcome: "failed",
        responseStarted: attemptError.responseStarted,
        ...(attemptError.responseId !== undefined ? { responseId: attemptError.responseId } : {}),
        failure,
        ...(retryDelayMs !== undefined ? { retryDelayMs } : {}),
      });
      if (retryDelayMs === undefined) throw attemptError.rootCause;
      emit(input.request.onProgress, {
        type: "model_retry",
        runId: input.runId,
        failedAttempt: sequence,
        nextAttempt: sequence + 1,
        delayMs: retryDelayMs,
        failureCode: failure.code,
      });
      await input.sleep(retryDelayMs, input.request.signal);
    }
  }
  throw new OperationStageError("INTERNAL_FAILURE", "Retry loop ended without a result");
}

async function collectModelStream(input: {
  readonly provider: ReturnType<OperationRunnerPorts["providers"]["provider"]>;
  readonly request: ExecuteOperationRequest;
  readonly packet: ContextPacket;
  readonly runId: OperationRunId;
}): Promise<CollectedStream> {
  const maxResponseBytes = input.request.maxResponseBytes ?? defaultMaxResponseBytes;
  const modelRequest = {
    model: input.request.model,
    messages: renderOperationMessages(input.request.intent, input.packet),
    maxOutputTokens: input.request.intent.output.maxTokens,
    temperature: input.request.temperature,
    topP: input.request.topP,
    reasoning: input.request.reasoning,
    responseFormat: input.provider.profile.capabilities.jsonObject ? "json_object" as const : "text" as const,
  };
  let responseId: string | undefined;
  let model: string | undefined;
  let text = "";
  let byteLength = 0;
  let usage: ModelUsage | undefined;
  let finishReason: FinishReason | undefined;
  try {
    for await (const event of input.provider.stream(modelRequest, {
      signal: input.request.signal,
      timeoutMs: input.request.timeoutMs,
    })) {
      throwIfCancelled(input.request.signal);
      if (event.type === "start") {
        if (responseId !== undefined || event.providerId !== input.request.providerId) {
          throw new OperationStageError("PROVIDER_FAILED", "Provider emitted an invalid or duplicate start event");
        }
        responseId = event.id;
        model = event.model;
        emit(input.request.onProgress, {
          type: "model_start",
          runId: input.runId,
          responseId,
          providerId: event.providerId,
          model,
        });
        continue;
      }
      if (event.type === "usage") {
        usage = event.usage;
        emit(input.request.onProgress, { type: "model_usage", runId: input.runId, usage });
        continue;
      }
      requireStreamIdentity(responseId, model);
      if (event.type === "tool_call_delta") {
        throw new OperationStageError("UNEXPECTED_TOOL_CALL", "Operation models must not invoke tools");
      }
      if (event.type === "reasoning_delta") {
        emit(input.request.onProgress, {
          type: "model_reasoning_delta",
          runId: input.runId,
          text: event.text,
        });
        continue;
      }
      if (event.type === "text_delta") {
        byteLength += new TextEncoder().encode(event.text).byteLength;
        if (byteLength > maxResponseBytes) {
          throw new OperationStageError(
            "MODEL_OUTPUT_TOO_LARGE",
            `Model output exceeds the configured ${maxResponseBytes}-byte limit`,
          );
        }
        text += event.text;
        emit(input.request.onProgress, {
          type: "model_text_delta",
          runId: input.runId,
          text: event.text,
        });
        continue;
      }
      if (event.type === "finish") {
        if (finishReason !== undefined) {
          throw new OperationStageError("PROVIDER_FAILED", "Provider emitted duplicate finish events");
        }
        finishReason = event.reason;
      }
    }
  } catch (error) {
    throw new ModelAttemptError(error, responseId);
  }
  try {
    const identity = requireStreamIdentity(responseId, model);
    if (finishReason === undefined) {
      throw new OperationStageError("PROVIDER_FAILED", "Provider stream ended without a finish event");
    }
    return {
      responseId: identity.responseId,
      model: identity.model,
      text,
      ...(usage !== undefined ? { usage } : {}),
      finishReason,
    };
  } catch (error) {
    throw new ModelAttemptError(error, responseId);
  }
}

function normalizeRetryPolicy(
  policy: ExecuteOperationRequest["retryPolicy"],
): Readonly<{ maxAttempts: number; baseDelayMs: number; maxDelayMs: number }> {
  const value = policy ?? defaultRetryPolicy;
  if (
    !Number.isInteger(value.maxAttempts)
    || value.maxAttempts < 1
    || value.maxAttempts > 3
    || !Number.isInteger(value.baseDelayMs)
    || value.baseDelayMs < 0
    || value.baseDelayMs > 5_000
    || !Number.isInteger(value.maxDelayMs)
    || value.maxDelayMs < value.baseDelayMs
    || value.maxDelayMs > 10_000
  ) {
    throw new OperationStageError(
      "PROVIDER_FAILED",
      "Retry policy must use 1..=3 attempts and bounded non-negative delays",
    );
  }
  return value;
}

function retryDelayFor(input: {
  readonly error: unknown;
  readonly responseStarted: boolean;
  readonly sequence: number;
  readonly policy: Readonly<{ maxAttempts: number; baseDelayMs: number; maxDelayMs: number }>;
}): number | undefined {
  if (
    !(input.error instanceof ProviderError)
    || !input.error.retriable
    || input.responseStarted
    || input.sequence >= input.policy.maxAttempts
    || input.error.kind === "cancelled"
    || (input.error.retryAfterMs ?? 0) > input.policy.maxDelayMs
  ) return undefined;
  const exponential = input.policy.baseDelayMs * (2 ** (input.sequence - 1));
  return Math.min(
    input.policy.maxDelayMs,
    Math.max(exponential, input.error.retryAfterMs ?? 0),
  );
}

function operationAttemptFailure(error: unknown): OperationAttemptFailure {
  if (error instanceof ProviderError) {
    const requestId = boundedAuditValue(error.requestId, 512);
    return {
      code: boundedAuditValue(error.code, 200)
        ?? `PROVIDER_${error.kind.toUpperCase()}`,
      kind: error.kind,
      ...(error.status !== undefined ? { status: error.status } : {}),
      ...(requestId !== undefined ? { requestId } : {}),
      ...(error.retryAfterMs !== undefined ? { retryAfterMs: error.retryAfterMs } : {}),
      retriable: error.retriable,
    };
  }
  return {
    code: error instanceof OperationStageError ? error.code : "PROVIDER_FAILED",
    retriable: false,
  };
}

function boundedAuditValue(value: string | undefined, maxLength: number): string | undefined {
  const normalized = value?.trim();
  return normalized ? Array.from(normalized).slice(0, maxLength).join("") : undefined;
}

function abortableSleep(milliseconds: number, signal?: AbortSignal): Promise<void> {
  if (signal?.aborted) {
    return Promise.reject(new OperationStageError("OPERATION_CANCELLED", "Operation was cancelled"));
  }
  return new Promise((resolve, reject) => {
    const timer = setTimeout(() => {
      signal?.removeEventListener("abort", onAbort);
      resolve();
    }, milliseconds);
    const onAbort = () => {
      clearTimeout(timer);
      reject(new OperationStageError("OPERATION_CANCELLED", "Operation was cancelled"));
    };
    signal?.addEventListener("abort", onAbort, { once: true });
  });
}

function assertFreshTarget(request: ExecuteOperationRequest): void {
  const { block, intent } = request;
  const target = intent.target;
  if (
    block.id !== target.blockId
    || block.documentId !== target.documentId
    || block.revision !== target.baseRevision
    || block.contentHash !== target.baseHash
  ) {
    throw new OperationStageError("TARGET_STALE", "Target block changed before the model request");
  }
  if (block.locked && intent.output.kind !== "findings") {
    throw new OperationStageError("TARGET_INVALID", "Target block is locked");
  }
  if (
    target.from.blockId !== block.id
    || target.to.blockId !== block.id
    || !validOffset(block.plainText, target.from.offset)
    || !validOffset(block.plainText, target.to.offset)
    || target.from.offset > target.to.offset
  ) {
    throw new OperationStageError("TARGET_INVALID", "Target range is not a valid UTF-16 range");
  }
}

async function assertTargetContext(
  packet: ContextPacket,
  request: ExecuteOperationRequest,
  hasher: OperationRunnerPorts["hasher"],
): Promise<void> {
  const { packetHash, ...packetWithoutHash } = packet;
  if (await hasher.sha256(stableStringify(packetWithoutHash)) !== packetHash) {
    throw new OperationStageError("CONTEXT_INVALID", "Context packet hash verification failed");
  }
  if (
    packet.operationIntentId !== request.intent.id
    || packet.projectId !== request.intent.projectId
    || packet.baseCommitId !== request.intent.baseCommitId
  ) {
    throw new OperationStageError("CONTEXT_INVALID", "Context packet is bound to a different operation");
  }
  const targetText = request.block.plainText.slice(
    request.intent.target.from.offset,
    request.intent.target.to.offset,
  );
  const targets = packet.items.filter((item) => item.tier === "L0_TARGET" && item.mandatory);
  if (
    targets.length !== 1
    || targets[0]?.renderMode !== "verbatim"
    || targets[0]?.content !== targetText
  ) {
    throw new OperationStageError(
      "CONTEXT_INVALID",
      "Context packet must contain exactly one mandatory verbatim L0 target matching the editor snapshot",
    );
  }
}

function assertResponseLimit(value: number | undefined): void {
  if (
    value !== undefined
    && (!Number.isInteger(value) || value < 1 || value > 16 * 1024 * 1024)
  ) {
    throw new OperationStageError(
      "MODEL_OUTPUT_TOO_LARGE",
      "maxResponseBytes must be an integer between 1 and 16 MiB",
    );
  }
}

function assertUsableFinishReason(reason: FinishReason): void {
  if (reason === "length") {
    throw new OperationStageError("MODEL_OUTPUT_TRUNCATED", "Model output was truncated by its token limit");
  }
  if (reason === "tool_calls") {
    throw new OperationStageError("UNEXPECTED_TOOL_CALL", "Operation model requested a tool call");
  }
  if (reason === "content_filter" || reason === "insufficient_system_resource") {
    throw new OperationStageError("PROVIDER_FAILED", `Provider stopped generation: ${reason}`);
  }
}

function requireStreamIdentity(
  responseId: string | undefined,
  model: string | undefined,
): { readonly responseId: string; readonly model: string } {
  if (!responseId || !model) {
    throw new OperationStageError("PROVIDER_FAILED", "Provider emitted output before a start event");
  }
  return { responseId, model };
}

function throwIfCancelled(signal: AbortSignal | undefined): void {
  if (signal?.aborted) {
    throw new OperationStageError("OPERATION_CANCELLED", "Operation was cancelled");
  }
}

function validOffset(text: string, offset: number): boolean {
  if (!Number.isInteger(offset) || offset < 0 || offset > text.length) return false;
  if (offset === 0 || offset === text.length) return true;
  const previous = text.charCodeAt(offset - 1);
  const next = text.charCodeAt(offset);
  return !(
    previous >= 0xd800
    && previous <= 0xdbff
    && next >= 0xdc00
    && next <= 0xdfff
  );
}

function emit(
  observer: ExecuteOperationRequest["onProgress"],
  event: OperationProgressEvent,
): void {
  try {
    observer?.(event);
  } catch {
    // Progress observers are UI-only and must not alter operation semantics.
  }
}

function classifyError(error: unknown): OperationExecutionErrorCode {
  if (error instanceof OperationStageError) return error.code;
  if (error instanceof ProviderError) return "PROVIDER_FAILED";
  if (error instanceof DomainError) return "CONTEXT_FAILED";
  if (error instanceof PatchEngineError) return "PATCH_FAILED";
  return "INTERNAL_FAILURE";
}

function safeReason(error: unknown): string {
  return error instanceof Error ? error.name : "UnknownError";
}

function safeMessage(error: unknown, cancelled: boolean): string {
  if (cancelled) return "Operation was cancelled";
  if (error instanceof OperationStageError || error instanceof ProviderError) return error.message;
  if (error instanceof DomainError) return "Context compilation failed";
  if (error instanceof PatchEngineError) return "Patch proposal compilation failed";
  return "Operation execution failed";
}
