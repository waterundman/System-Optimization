import type { PatchHunk, PatchProposal } from "@optimizer/protocol";
import { validatePatchProposal } from "@optimizer/protocol";
import { diffText } from "./diff.ts";
import { PatchEngineError } from "./errors.ts";
import { stableStringify } from "./stable-json.ts";
import type {
  CompilePatchProposalInput,
  PatchHasher,
} from "./types.ts";

export async function compilePatchProposal(
  input: CompilePatchProposalInput,
  hasher: PatchHasher,
): Promise<PatchProposal> {
  assertTarget(input);
  if (input.replacementText.length > 10_000_000 || input.block.plainText.length > 10_000_000) {
    throw new PatchEngineError(
      "INPUT_TOO_LARGE",
      "Patch compilation is limited to 10,000,000 UTF-16 code units",
    );
  }
  const sourceText = input.block.plainText.slice(
    input.target.from.offset,
    input.target.to.offset,
  );
  const diff = diffText(sourceText, input.replacementText, input.diffOptions);
  if (diff.changes.length === 0) {
    throw new PatchEngineError("NO_CHANGES", "The replacement is identical to the selected source text");
  }
  const maxHunks = input.maxHunks ?? 500;
  if (!Number.isInteger(maxHunks) || maxHunks < 1) {
    throw new RangeError("maxHunks must be a positive integer");
  }
  if (diff.changes.length > maxHunks) {
    throw new PatchEngineError(
      "TOO_MANY_HUNKS",
      `Diff produced ${diff.changes.length} hunks; maximum is ${maxHunks}`,
    );
  }

  const atomicGroup = input.atomic && diff.changes.length > 1
    ? `${input.id}:atomic`
    : undefined;
  const hunks: PatchHunk[] = diff.changes.map((change, index) => ({
    id: `${input.id}:h${index + 1}`,
    from: {
      blockId: input.target.blockId,
      offset: input.target.from.offset + change.from,
      affinity: "after",
    },
    to: {
      blockId: input.target.blockId,
      offset: input.target.from.offset + change.to,
      affinity: "before",
    },
    original: change.original,
    replacement: change.replacement,
    granularity: change.granularity,
    atomicGroup,
  }));
  const proposalWithoutHash = {
    schemaVersion: 2 as const,
    id: input.id,
    operationRunId: input.operationRunId,
    baseCommitId: input.baseCommitId,
    target: input.target,
    hunks,
    summary: input.summary,
    warnings: diff.warnings,
    status: "review" as const,
    createdAt: input.createdAt,
  };
  const proposal: PatchProposal = {
    ...proposalWithoutHash,
    proposalHash: await hasher.sha256(stableStringify(proposalWithoutHash)),
  };
  assertValidProposal(proposal);
  return proposal;
}

export async function verifyProposalHash(
  proposal: PatchProposal,
  hasher: PatchHasher,
): Promise<boolean> {
  const { proposalHash: _proposalHash, ...payload } = proposal;
  return proposal.proposalHash === await hasher.sha256(stableStringify(payload));
}

function assertTarget(input: CompilePatchProposalInput): void {
  const { block, target } = input;
  if (
    block.id !== target.blockId
    || block.documentId !== target.documentId
    || block.revision !== target.baseRevision
    || block.contentHash !== target.baseHash
    || target.from.blockId !== block.id
    || target.to.blockId !== block.id
  ) {
    throw new PatchEngineError(
      "TARGET_MISMATCH",
      "Proposal target does not match the supplied stable block snapshot",
    );
  }
  assertOffset(block.plainText, target.from.offset);
  assertOffset(block.plainText, target.to.offset);
  if (target.from.offset > target.to.offset) {
    throw new PatchEngineError("TARGET_RANGE_INVALID", "Target start must not exceed target end");
  }
}

function assertOffset(text: string, offset: number): void {
  if (
    !Number.isInteger(offset)
    || offset < 0
    || offset > text.length
    || splitsSurrogatePair(text, offset)
  ) {
    throw new PatchEngineError(
      "TARGET_RANGE_INVALID",
      `Offset ${offset} is not a valid UTF-16 boundary`,
    );
  }
}

function splitsSurrogatePair(text: string, offset: number): boolean {
  if (offset <= 0 || offset >= text.length) return false;
  const previous = text.charCodeAt(offset - 1);
  const next = text.charCodeAt(offset);
  return (
    previous >= 0xd800
    && previous <= 0xdbff
    && next >= 0xdc00
    && next <= 0xdfff
  );
}

function assertValidProposal(proposal: PatchProposal): void {
  const result = validatePatchProposal(proposal);
  if (!result.ok) {
    throw new PatchEngineError(
      "PROPOSAL_INVALID",
      "Compiled proposal violates the stable protocol",
      { issues: result.issues.map((issue) => `${issue.path}: ${issue.message}`).join("; ") },
    );
  }
}
