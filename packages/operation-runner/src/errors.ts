import type {
  ContextPacket,
  ModelProviderId,
  OperationRunId,
  OperationState,
} from "../../protocol/src/index.ts";
import type { OperationTransition } from "../../kernel/src/index.ts";
import type { OperationAttempt } from "./types.ts";

export type OperationExecutionErrorCode =
  | "OPERATION_CANCELLED"
  | "TARGET_STALE"
  | "TARGET_INVALID"
  | "CONTEXT_INVALID"
  | "MODEL_OUTPUT_INVALID"
  | "MODEL_OUTPUT_TOO_LARGE"
  | "MODEL_OUTPUT_TRUNCATED"
  | "UNEXPECTED_TOOL_CALL"
  | "PROVIDER_FAILED"
  | "CONTEXT_FAILED"
  | "PATCH_FAILED"
  | "INTERNAL_FAILURE";

export class OperationStageError extends Error {
  readonly code: OperationExecutionErrorCode;

  constructor(code: OperationExecutionErrorCode, message: string) {
    super(message);
    this.name = "OperationStageError";
    this.code = code;
  }
}

export class OperationExecutionError extends Error {
  readonly code: OperationExecutionErrorCode;
  readonly runId: OperationRunId;
  readonly state: Extract<OperationState, "failed" | "cancelled">;
  readonly history: readonly OperationTransition[];
  readonly providerId: ModelProviderId;
  readonly model?: string;
  readonly contextPacket?: ContextPacket;
  readonly attempts: readonly OperationAttempt[];
  readonly rootCause: unknown;

  constructor(input: {
    readonly code: OperationExecutionErrorCode;
    readonly message: string;
    readonly runId: OperationRunId;
    readonly state: Extract<OperationState, "failed" | "cancelled">;
    readonly history: readonly OperationTransition[];
    readonly providerId: ModelProviderId;
    readonly model?: string;
    readonly contextPacket?: ContextPacket;
    readonly attempts?: readonly OperationAttempt[];
    readonly rootCause: unknown;
  }) {
    super(input.message);
    this.name = "OperationExecutionError";
    this.code = input.code;
    this.runId = input.runId;
    this.state = input.state;
    this.history = [...input.history];
    this.providerId = input.providerId;
    this.model = input.model;
    this.contextPacket = input.contextPacket;
    this.attempts = [...(input.attempts ?? [])];
    this.rootCause = input.rootCause;
  }
}
