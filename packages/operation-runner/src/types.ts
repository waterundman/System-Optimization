import type {
  ContextPacket,
  ModelProviderId,
  OperationIntent,
  OperationRunId,
  PatchProposal,
  PatchProposalId,
} from "../../protocol/src/index.ts";
import type { EditorBlockSnapshot } from "../../editor-bridge/src/index.ts";
import type {
  Clock,
  CompileContextRequest,
  ContentHasher,
  OperationTransition,
} from "../../kernel/src/index.ts";
import type {
  FinishReason,
  ModelProvider,
  ModelUsage,
  ReasoningOptions,
} from "../../model-gateway/src/index.ts";

export interface OperationRunnerIds {
  nextRunId(): OperationRunId;
  nextProposalId(): PatchProposalId;
}

export interface ContextCompilationPort {
  compile(request: CompileContextRequest): Promise<ContextPacket>;
}

export interface ModelProviderLookup {
  provider(providerId: ModelProviderId): ModelProvider;
}

export interface OperationRunnerPorts {
  readonly contextCompiler: ContextCompilationPort;
  readonly providers: ModelProviderLookup;
  readonly hasher: ContentHasher;
  readonly clock: Clock;
  readonly ids: OperationRunnerIds;
}

export type FindingSeverity = "info" | "warning" | "error";

export interface OperationFinding {
  readonly severity: FindingSeverity;
  readonly message: string;
  readonly sourceRef?: string;
}

export type OperationProgressEvent =
  | {
      readonly type: "lifecycle";
      readonly runId: OperationRunId;
      readonly transition: OperationTransition;
    }
  | {
      readonly type: "model_start";
      readonly runId: OperationRunId;
      readonly responseId: string;
      readonly providerId: ModelProviderId;
      readonly model: string;
    }
  | {
      readonly type: "model_text_delta" | "model_reasoning_delta";
      readonly runId: OperationRunId;
      readonly text: string;
    }
  | {
      readonly type: "model_usage";
      readonly runId: OperationRunId;
      readonly usage: ModelUsage;
    };

export interface ExecuteOperationRequest {
  readonly intent: OperationIntent;
  readonly providerId: ModelProviderId;
  readonly block: EditorBlockSnapshot;
  readonly modelLimit: number;
  readonly reservedOverhead?: number;
  readonly model?: string;
  readonly timeoutMs?: number;
  readonly maxResponseBytes?: number;
  readonly temperature?: number;
  readonly topP?: number;
  readonly reasoning?: ReasoningOptions;
  readonly atomicPatch?: boolean;
  readonly signal?: AbortSignal;
  readonly onProgress?: (event: OperationProgressEvent) => void;
}

interface OperationExecutionBase {
  readonly runId: OperationRunId;
  readonly providerId: ModelProviderId;
  readonly model: string;
  readonly responseId: string;
  readonly contextPacket: ContextPacket;
  readonly usage?: ModelUsage;
  readonly finishReason: FinishReason;
  readonly history: readonly OperationTransition[];
}

export type OperationExecutionResult =
  | (OperationExecutionBase & {
      readonly kind: "patch_proposal";
      readonly proposal: PatchProposal;
    })
  | (OperationExecutionBase & {
      readonly kind: "findings";
      readonly findings: readonly OperationFinding[];
      readonly summary?: string;
    });

export interface ParsedReplacementOutput {
  readonly kind: "replacement";
  readonly replacementText: string;
  readonly summary?: string;
}

export interface ParsedFindingsOutput {
  readonly kind: "findings";
  readonly findings: readonly OperationFinding[];
  readonly summary?: string;
}

export type ParsedModelOutput = ParsedReplacementOutput | ParsedFindingsOutput;
