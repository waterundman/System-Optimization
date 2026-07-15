import type {
  OperationIntent,
  OperationPersistenceBundleV1,
  PersistReviewEventCommandV1,
} from "../../protocol/src/index.ts";
import {
  stableStringify,
  type ContentHasher,
  type OperationTransition,
} from "../../kernel/src/index.ts";
import { ProviderError } from "../../model-gateway/src/index.ts";
import { OperationExecutionError, OperationStageError } from "./errors.ts";
import type { OperationExecutionResult } from "./types.ts";

export interface SuccessfulPersistenceInput {
  readonly intent: OperationIntent;
  readonly result: OperationExecutionResult;
  readonly startedAt: string;
  readonly findingsArtifactId?: string;
}

export interface FailedPersistenceInput {
  readonly intent: OperationIntent;
  readonly error: OperationExecutionError;
  readonly startedAt: string;
  readonly requestedModel?: string;
}

export async function buildSuccessfulPersistenceBundle(
  input: SuccessfulPersistenceInput,
  hasher: ContentHasher,
): Promise<OperationPersistenceBundleV1> {
  assertBinding(input.intent, input.result.contextPacket.operationIntentId, input.result.contextPacket.projectId, input.result.contextPacket.baseCommitId);
  assertHistory(input.result.history, "review");
  const updatedAt = lastOccurredAt(input.result.history);
  const artifact = input.result.kind === "patch_proposal"
    ? {
        id: input.result.proposal.id,
        kind: "patch_proposal" as const,
        bindingHash: input.result.proposal.proposalHash,
        payload: input.result.proposal as unknown as Readonly<Record<string, unknown>>,
        createdAt: input.result.proposal.createdAt,
      }
    : await findingsArtifact(input, updatedAt, hasher);
  return {
    schemaVersion: 1,
    run: {
      id: input.result.runId,
      operationIntentId: input.intent.id,
      projectId: input.intent.projectId,
      baseCommitId: input.intent.baseCommitId,
      providerId: input.result.providerId,
      model: required("result.model", input.result.model),
      state: "review",
      responseId: required("result.responseId", input.result.responseId),
      finishReason: input.result.finishReason,
      ...(input.result.usage !== undefined ? { usage: input.result.usage } : {}),
      startedAt: required("startedAt", input.startedAt),
      updatedAt,
    },
    contextPacket: contextRecord(input.result.contextPacket),
    lifecycleEvents: lifecycleRecords(input.result.history),
    artifact,
  };
}

export function buildFailedPersistenceBundle(
  input: FailedPersistenceInput,
): OperationPersistenceBundleV1 {
  assertHistory(input.error.history, input.error.state);
  if (input.error.contextPacket) {
    assertBinding(
      input.intent,
      input.error.contextPacket.operationIntentId,
      input.error.contextPacket.projectId,
      input.error.contextPacket.baseCommitId,
    );
  }
  const rootCause = input.error.rootCause;
  const retriable = rootCause instanceof ProviderError ? rootCause.retriable : false;
  return {
    schemaVersion: 1,
    run: {
      id: input.error.runId,
      operationIntentId: input.intent.id,
      projectId: input.intent.projectId,
      baseCommitId: input.intent.baseCommitId,
      providerId: input.error.providerId,
      model: firstNonEmpty(input.error.model, input.requestedModel) ?? "unresolved",
      state: input.error.state,
      failure: {
        code: input.error.code,
        message: input.error.message,
        retriable,
      },
      startedAt: required("startedAt", input.startedAt),
      updatedAt: lastOccurredAt(input.error.history),
    },
    ...(input.error.contextPacket !== undefined
      ? { contextPacket: contextRecord(input.error.contextPacket) }
      : {}),
    lifecycleEvents: lifecycleRecords(input.error.history),
  };
}

export function serializeOperationPersistenceBundle(
  bundle: OperationPersistenceBundleV1,
): string {
  return stableStringify(bundle);
}

export function serializeReviewEventCommand(command: PersistReviewEventCommandV1): string {
  return stableStringify(command);
}

async function findingsArtifact(
  input: SuccessfulPersistenceInput,
  createdAt: string,
  hasher: ContentHasher,
): Promise<OperationPersistenceBundleV1["artifact"]> {
  if (input.result.kind !== "findings") {
    throw new OperationStageError("INTERNAL_FAILURE", "Expected findings result");
  }
  const id = required("findingsArtifactId", input.findingsArtifactId);
  const payload = {
    schemaVersion: 1,
    kind: "findings",
    findings: input.result.findings,
    ...(input.result.summary !== undefined ? { summary: input.result.summary } : {}),
  } as const;
  return {
    id,
    kind: "findings",
    bindingHash: await hasher.sha256(stableStringify(payload)),
    payload,
    createdAt,
  };
}

function contextRecord(
  packet: OperationExecutionResult["contextPacket"],
): NonNullable<OperationPersistenceBundleV1["contextPacket"]> {
  return {
    id: packet.id,
    operationIntentId: packet.operationIntentId,
    projectId: packet.projectId,
    baseCommitId: packet.baseCommitId,
    packetHash: packet.packetHash,
    payload: packet,
    createdAt: packet.compiledAt,
  };
}

function lifecycleRecords(
  history: readonly OperationTransition[],
): OperationPersistenceBundleV1["lifecycleEvents"] {
  return history.map((transition) => ({
    fromState: transition.from,
    toState: transition.to,
    occurredAt: transition.occurredAt,
    ...(transition.reason !== undefined ? { reason: transition.reason } : {}),
  }));
}

function assertBinding(
  intent: OperationIntent,
  operationIntentId: string,
  projectId: string,
  baseCommitId: string,
): void {
  if (
    operationIntentId !== intent.id
    || projectId !== intent.projectId
    || baseCommitId !== intent.baseCommitId
  ) {
    throw new OperationStageError(
      "CONTEXT_INVALID",
      "Persistence input is bound to a different operation, project or commit",
    );
  }
}

function assertHistory(
  history: readonly OperationTransition[],
  expectedState: string,
): void {
  if (history.length === 0 || history.at(-1)?.to !== expectedState) {
    throw new OperationStageError(
      "INTERNAL_FAILURE",
      "Operation history does not end in the persisted state",
    );
  }
}

function lastOccurredAt(history: readonly OperationTransition[]): string {
  return required("history.occurredAt", history.at(-1)?.occurredAt);
}

function required(field: string, value: string | undefined): string {
  if (!value?.trim()) {
    throw new OperationStageError("INTERNAL_FAILURE", `${field} must not be empty`);
  }
  return value;
}

function firstNonEmpty(...values: readonly (string | undefined)[]): string | undefined {
  for (const value of values) {
    if (value?.trim()) return value.trim();
  }
  return undefined;
}
