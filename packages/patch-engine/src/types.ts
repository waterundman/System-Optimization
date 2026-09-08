import type {
  CommitId,
  DiffGranularity,
  OperationRunId,
  OperationTarget,
  PatchProposal,
  PatchProposalId,
} from "@optimizer/protocol";
import type {
  EditorBlockSnapshot,
  EditorDocumentSnapshot,
  EditorTransaction,
} from "@optimizer/editor-bridge";

export type DiffSegmentKind = "equal" | "delete" | "insert";

export interface DiffSegment {
  readonly kind: DiffSegmentKind;
  readonly text: string;
  readonly granularity: DiffGranularity;
  readonly oldFrom: number;
  readonly oldTo: number;
  readonly newFrom: number;
  readonly newTo: number;
}

export interface TextDiff {
  readonly segments: readonly DiffSegment[];
  readonly changes: readonly TextChange[];
  readonly warnings: readonly string[];
}

export interface TextChange {
  readonly from: number;
  readonly to: number;
  readonly original: string;
  readonly replacement: string;
  readonly granularity: DiffGranularity;
}

export interface DiffOptions {
  readonly maxMatrixCells?: number;
}

export interface PatchHasher {
  readonly id: string;
  sha256(value: string): Promise<string>;
}

export interface CompilePatchProposalInput {
  readonly id: PatchProposalId;
  readonly operationRunId: OperationRunId;
  readonly baseCommitId: CommitId;
  readonly target: OperationTarget;
  readonly block: EditorBlockSnapshot;
  readonly replacementText: string;
  readonly createdAt: string;
  readonly summary?: string;
  readonly atomic?: boolean;
  readonly maxHunks?: number;
  readonly diffOptions?: DiffOptions;
}

export type HunkDecision = "pending" | "accepted" | "rejected";
export type ReviewSessionStatus = "review" | "ready" | "applied" | "rejected" | "conflicted";

export interface PatchReviewSession {
  readonly proposalId: PatchProposalId;
  readonly proposalHash: string;
  readonly revision: number;
  readonly status: ReviewSessionStatus;
  readonly decisions: Readonly<Record<string, HunkDecision>>;
}

export interface ReviewDecisionCommand {
  readonly hunkId: string;
  readonly decision: Exclude<HunkDecision, "pending">;
  readonly expectedRevision: number;
}

export interface BlockContentAdapter {
  replacePlainText(
    block: EditorBlockSnapshot,
    nextPlainText: string,
  ): Readonly<Record<string, unknown>>;
}

export type ReviewConflictCode =
  | "TARGET_BLOCK_MISSING"
  | "TARGET_BLOCK_CHANGED"
  | "HUNK_SOURCE_CHANGED";

export interface ReviewConflict {
  readonly code: ReviewConflictCode;
  readonly hunkId?: string;
  readonly message: string;
}

export type ReviewCompilation =
  | {
      readonly status: "ready_to_apply";
      readonly transaction: EditorTransaction;
      readonly nextPlainText: string;
      readonly acceptedHunkIds: readonly string[];
      readonly rejectedHunkIds: readonly string[];
    }
  | {
      readonly status: "rejected";
      readonly rejectedHunkIds: readonly string[];
    }
  | {
      readonly status: "conflicted";
      readonly conflicts: readonly ReviewConflict[];
    };

export interface CompileReviewInput {
  readonly proposal: PatchProposal;
  readonly session: PatchReviewSession;
  readonly document: EditorDocumentSnapshot;
  readonly transactionId: string;
  readonly contentAdapter: BlockContentAdapter;
  readonly hasher: PatchHasher;
}
