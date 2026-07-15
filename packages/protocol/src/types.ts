export type Brand<T, Name extends string> = T & { readonly __brand: Name };

export type WorkspaceId = Brand<string, "WorkspaceId">;
export type ProjectId = Brand<string, "ProjectId">;
export type DocumentId = Brand<string, "DocumentId">;
export type BlockId = Brand<string, "BlockId">;
export type CommitId = Brand<string, "CommitId">;
export type OperationIntentId = Brand<string, "OperationIntentId">;
export type OperationRunId = Brand<string, "OperationRunId">;
export type ContextPacketId = Brand<string, "ContextPacketId">;
export type PatchProposalId = Brand<string, "PatchProposalId">;
export type EventId = Brand<string, "EventId">;
export type CredentialRef = Brand<string, "CredentialRef">;
export type ModelProviderId = "deepseek" | "qwen" | "kimi" | "minimax" | "ollama";
export type QwenDeploymentRegion = "china" | "singapore" | "us" | "germany" | "japan";

export type OperationType =
  | "polish"
  | "continue_scene"
  | "compress"
  | "expand"
  | "critique";

export type OperationStrength = "low" | "medium" | "high";
export type OutputKind = "patch_proposal" | "insert_proposal" | "findings";
export type ProviderLocality = "local" | "remote";

export interface TextAnchor {
  readonly blockId: BlockId;
  readonly offset: number;
  readonly affinity?: "before" | "after";
}

export interface OperationTarget {
  readonly documentId: DocumentId;
  readonly blockId: BlockId;
  readonly baseRevision: number;
  readonly baseHash: string;
  readonly from: TextAnchor;
  readonly to: TextAnchor;
}

export interface OperationConstraint {
  readonly severity: "hard" | "soft";
  readonly rule: string;
  readonly sourceRef?: string;
}

export interface OperationIntent {
  readonly schemaVersion: 1;
  readonly id: OperationIntentId;
  readonly projectId: ProjectId;
  readonly baseCommitId: CommitId;
  readonly type: OperationType;
  readonly strength: OperationStrength;
  readonly target: OperationTarget;
  readonly userInstruction?: string;
  readonly constraints: readonly OperationConstraint[];
  readonly output: {
    readonly kind: OutputKind;
    readonly maxTokens: number;
  };
  readonly createdAt: string;
}

export type ContextTier =
  | "L0_TARGET"
  | "L1_LOCAL"
  | "L2_STRUCTURAL"
  | "L3_KNOWLEDGE"
  | "L4_STYLE_GLOBAL";

export type CanonicalStatus =
  | "canonical"
  | "draft"
  | "disputed"
  | "archived"
  | "rejected"
  | "deleted";

export type Authority =
  | "user_confirmed"
  | "source_derived"
  | "model_inferred"
  | "external_untrusted";

export type Sensitivity = "public" | "local" | "local_sensitive" | "never_send";
export type RenderMode = "verbatim" | "summary" | "constraint";

export interface ContextItem {
  readonly id: string;
  readonly sourceRef: string;
  readonly sourceHash: string;
  readonly tier: ContextTier;
  readonly status: CanonicalStatus;
  readonly authority: Authority;
  readonly sensitivity: Sensitivity;
  readonly renderMode: RenderMode;
  readonly reasonCodes: readonly string[];
  readonly estimatedTokens: number;
  readonly score: number;
  readonly mandatory: boolean;
  readonly content: string;
}

export interface ContextExclusion {
  readonly sourceRef: string;
  readonly sourceHash: string;
  readonly reason:
    | "INELIGIBLE_STATUS"
    | "DRAFT_NOT_SELECTED"
    | "POLICY_DENIED"
    | "DUPLICATE"
    | "TIER_BUDGET"
    | "TOTAL_BUDGET";
}

export interface ContextBudget {
  readonly modelLimit: number;
  readonly reservedOutput: number;
  readonly reservedOverhead: number;
  readonly inputBudget: number;
  readonly estimatedInput: number;
  readonly tokenizer: string;
}

export interface ContextPacket {
  readonly schemaVersion: 1;
  readonly id: ContextPacketId;
  readonly operationIntentId: OperationIntentId;
  readonly projectId: ProjectId;
  readonly baseCommitId: CommitId;
  readonly providerLocality: ProviderLocality;
  readonly items: readonly ContextItem[];
  readonly exclusions: readonly ContextExclusion[];
  readonly budget: ContextBudget;
  readonly compilerVersion: string;
  readonly operationProfileVersion: string;
  readonly compiledAt: string;
  readonly packetHash: string;
}

export type PatchProposalStatus = "review" | "accepted" | "rejected" | "conflicted";
export type DiffGranularity = "paragraph" | "sentence" | "token";

export interface PatchHunk {
  readonly id: string;
  readonly from: TextAnchor;
  readonly to: TextAnchor;
  readonly original: string;
  readonly replacement: string;
  readonly granularity: DiffGranularity;
  readonly atomicGroup?: string;
}

export interface PatchProposal {
  readonly schemaVersion: 2;
  readonly id: PatchProposalId;
  readonly operationRunId: OperationRunId;
  readonly baseCommitId: CommitId;
  readonly target: OperationTarget;
  readonly hunks: readonly PatchHunk[];
  readonly summary?: string;
  readonly warnings: readonly string[];
  readonly status: PatchProposalStatus;
  readonly createdAt: string;
  readonly proposalHash: string;
}

export interface EventEnvelope<Payload = unknown> {
  readonly schemaVersion: 1;
  readonly id: EventId;
  readonly type: string;
  readonly occurredAt: string;
  readonly traceId: string;
  readonly projectId?: ProjectId;
  readonly operationRunId?: OperationRunId;
  readonly sequence?: number;
  readonly payload: Payload;
}

export interface ModelProviderConfiguration {
  readonly schemaVersion: 1;
  readonly id: string;
  readonly providerId: ModelProviderId;
  readonly enabled: boolean;
  readonly defaultModel: string;
  readonly credentialRef: CredentialRef;
  readonly qwen?: {
    readonly region: QwenDeploymentRegion;
    readonly workspaceId?: string;
  };
  readonly defaultTimeoutMs: number;
  readonly maxRequestBytes: number;
  readonly updatedAt: string;
}

export interface PersistedModelUsage {
  readonly inputTokens: number;
  readonly outputTokens: number;
  readonly totalTokens: number;
  readonly cachedInputTokens?: number;
  readonly reasoningTokens?: number;
}

export interface PersistedOperationFailure {
  readonly code: string;
  readonly message: string;
  readonly retriable: boolean;
}

export interface OperationPersistenceBundleV1 {
  readonly schemaVersion: 1;
  readonly run: {
    readonly id: OperationRunId;
    readonly operationIntentId: OperationIntentId;
    readonly projectId: ProjectId;
    readonly baseCommitId: CommitId;
    readonly providerId: ModelProviderId;
    readonly model: string;
    readonly state: Extract<OperationState, "review" | "failed" | "cancelled">;
    readonly responseId?: string;
    readonly finishReason?: string;
    readonly usage?: PersistedModelUsage;
    readonly failure?: PersistedOperationFailure;
    readonly startedAt: string;
    readonly updatedAt: string;
  };
  readonly contextPacket?: {
    readonly id: ContextPacketId;
    readonly operationIntentId: OperationIntentId;
    readonly projectId: ProjectId;
    readonly baseCommitId: CommitId;
    readonly packetHash: string;
    readonly payload: ContextPacket;
    readonly createdAt: string;
  };
  readonly lifecycleEvents: readonly {
    readonly fromState: OperationState;
    readonly toState: OperationState;
    readonly occurredAt: string;
    readonly reason?: string;
  }[];
  readonly artifact?: {
    readonly id: string;
    readonly kind: "patch_proposal" | "findings";
    readonly bindingHash: string;
    readonly payload: Readonly<Record<string, unknown>>;
    readonly createdAt: string;
  };
}

export type PersistedReviewStatus = "review" | "ready" | "applied" | "rejected" | "conflicted";

export interface PersistReviewEventCommandV1 {
  readonly schemaVersion: 1;
  readonly id: string;
  readonly proposalId: PatchProposalId;
  readonly expectedRevision: number;
  readonly expectedStatus: PersistedReviewStatus;
  readonly kind: "decision" | "apply" | "conflict" | "rebase" | "reject";
  readonly nextStatus: PersistedReviewStatus;
  readonly hunkId?: string;
  readonly decision?: "accepted" | "rejected";
  readonly payload?: unknown;
  readonly occurredAt: string;
}

export type OperationState =
  | "draft"
  | "compiling"
  | "preflight"
  | "queued"
  | "streaming"
  | "validating"
  | "review"
  | "accepted"
  | "rejected"
  | "conflicted"
  | "failed"
  | "cancelled";
