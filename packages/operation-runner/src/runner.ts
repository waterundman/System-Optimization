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
  OperationExecutionResult,
  OperationProgressEvent,
  OperationRunnerPorts,
} from "./types.ts";

const defaultReservedOverhead = 800;
const defaultMaxResponseBytes = 10 * 1024 * 1024;

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
      const collected = await collectModelStream({
        provider,
        request,
        packet,
        runId,
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
