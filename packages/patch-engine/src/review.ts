import type { PatchHunk, PatchProposal } from "../../protocol/src/index.ts";
import { validatePatchProposal } from "../../protocol/src/index.ts";
import { PatchEngineError } from "./errors.ts";
import { verifyProposalHash } from "./proposal.ts";
import type {
  CompileReviewInput,
  HunkDecision,
  PatchHasher,
  PatchReviewSession,
  ReviewCompilation,
  ReviewDecisionCommand,
  ReviewSessionStatus,
} from "./types.ts";

export async function createPatchReview(
  proposal: PatchProposal,
  hasher: PatchHasher,
): Promise<PatchReviewSession> {
  await assertProposalIntegrity(proposal, hasher);
  return {
    proposalId: proposal.id,
    proposalHash: proposal.proposalHash,
    revision: 0,
    status: "review",
    decisions: Object.fromEntries(proposal.hunks.map((hunk) => [hunk.id, "pending"])),
  };
}

export function decidePatchHunk(
  proposal: PatchProposal,
  session: PatchReviewSession,
  command: ReviewDecisionCommand,
): PatchReviewSession {
  assertBinding(proposal, session);
  if (session.status !== "review" && session.status !== "ready") {
    throw new PatchEngineError("REVIEW_FINALIZED", `Review is already ${session.status}`);
  }
  if (session.revision !== command.expectedRevision) {
    throw new PatchEngineError(
      "PROPOSAL_BINDING_MISMATCH",
      "Review decision was created from a stale session revision",
      { expectedRevision: command.expectedRevision, actualRevision: session.revision },
    );
  }
  const selected = proposal.hunks.find((hunk) => hunk.id === command.hunkId);
  if (!selected) {
    throw new PatchEngineError("HUNK_NOT_FOUND", `Hunk not found: ${command.hunkId}`);
  }

  const affectedIds = selected.atomicGroup
    ? proposal.hunks
        .filter((hunk) => hunk.atomicGroup === selected.atomicGroup)
        .map((hunk) => hunk.id)
    : [selected.id];
  const decisions: Record<string, HunkDecision> = { ...session.decisions };
  for (const id of affectedIds) decisions[id] = command.decision;
  const status = Object.values(decisions).every((decision) => decision !== "pending")
    ? "ready"
    : "review";
  return {
    ...session,
    revision: session.revision + 1,
    status,
    decisions,
  };
}

export function finalizePatchReview(
  session: PatchReviewSession,
  status: Extract<ReviewSessionStatus, "applied" | "rejected" | "conflicted">,
  expectedRevision: number,
): PatchReviewSession {
  if (session.revision !== expectedRevision) {
    throw new PatchEngineError(
      "PROPOSAL_BINDING_MISMATCH",
      "Review finalization was created from a stale session revision",
    );
  }
  if (session.status !== "ready") {
    throw new PatchEngineError("REVIEW_INCOMPLETE", "Only a ready review can be finalized");
  }
  return { ...session, revision: session.revision + 1, status };
}

export async function compileReviewTransaction(
  input: CompileReviewInput,
): Promise<ReviewCompilation> {
  const { proposal, session, document } = input;
  assertBinding(proposal, session);
  await assertProposalIntegrity(proposal, input.hasher);
  if (session.status !== "ready") {
    throw new PatchEngineError("REVIEW_INCOMPLETE", "Every hunk must be accepted or rejected");
  }

  const accepted = proposal.hunks.filter((hunk) => session.decisions[hunk.id] === "accepted");
  const rejected = proposal.hunks.filter((hunk) => session.decisions[hunk.id] === "rejected");
  if (accepted.length === 0) {
    return { status: "rejected", rejectedHunkIds: rejected.map((hunk) => hunk.id) };
  }

  const block = document.blocks.find((candidate) => candidate.id === proposal.target.blockId);
  if (!block || document.documentId !== proposal.target.documentId) {
    return {
      status: "conflicted",
      conflicts: [{ code: "TARGET_BLOCK_MISSING", message: "The proposal target block no longer exists" }],
    };
  }
  if (
    block.revision !== proposal.target.baseRevision
    || block.contentHash !== proposal.target.baseHash
    || block.locked
  ) {
    return {
      status: "conflicted",
      conflicts: [{ code: "TARGET_BLOCK_CHANGED", message: "The target block changed or became locked" }],
    };
  }

  const sourceConflicts = proposal.hunks
    .filter((hunk) => block.plainText.slice(hunk.from.offset, hunk.to.offset) !== hunk.original)
    .map((hunk) => ({
      code: "HUNK_SOURCE_CHANGED" as const,
      hunkId: hunk.id,
      message: `Source text changed for hunk ${hunk.id}`,
    }));
  if (sourceConflicts.length > 0) {
    return { status: "conflicted", conflicts: sourceConflicts };
  }

  const nextPlainText = applyAcceptedHunks(block.plainText, accepted);
  return {
    status: "ready_to_apply",
    transaction: {
      id: input.transactionId,
      documentId: document.documentId,
      expectedDocumentRevision: document.revision,
      steps: [{
        type: "replace_block",
        blockId: block.id,
        expectedRevision: block.revision,
        expectedHash: block.contentHash,
        nextContent: input.contentAdapter.replacePlainText(block, nextPlainText),
        nextPlainText,
      }],
    },
    nextPlainText,
    acceptedHunkIds: accepted.map((hunk) => hunk.id),
    rejectedHunkIds: rejected.map((hunk) => hunk.id),
  };
}

function applyAcceptedHunks(source: string, hunks: readonly PatchHunk[]): string {
  let result = source;
  const descending = [...hunks].sort(
    (left, right) => right.from.offset - left.from.offset || right.to.offset - left.to.offset,
  );
  for (const hunk of descending) {
    result = result.slice(0, hunk.from.offset) + hunk.replacement + result.slice(hunk.to.offset);
  }
  return result;
}

async function assertProposalIntegrity(
  proposal: PatchProposal,
  hasher: PatchHasher,
): Promise<void> {
  const validation = validatePatchProposal(proposal);
  if (
    proposal.status !== "review"
    || !validation.ok
    || !(await verifyProposalHash(proposal, hasher))
  ) {
    throw new PatchEngineError("PROPOSAL_INVALID", "Proposal validation or integrity check failed");
  }
}

function assertBinding(proposal: PatchProposal, session: PatchReviewSession): void {
  if (
    session.proposalId !== proposal.id
    || session.proposalHash !== proposal.proposalHash
    || proposal.hunks.some((hunk) => !(hunk.id in session.decisions))
    || Object.keys(session.decisions).some((id) => !proposal.hunks.some((hunk) => hunk.id === id))
    || Object.values(session.decisions).some(
      (decision) => !["pending", "accepted", "rejected"].includes(decision),
    )
  ) {
    throw new PatchEngineError(
      "PROPOSAL_BINDING_MISMATCH",
      "Review session does not match this immutable proposal",
    );
  }
}
