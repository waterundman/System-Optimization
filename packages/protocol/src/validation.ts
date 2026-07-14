import type {
  DiffGranularity,
  OperationIntent,
  OperationType,
  OutputKind,
  PatchProposal,
  PatchProposalStatus,
} from "./types.ts";

export interface ValidationIssue {
  readonly path: string;
  readonly message: string;
}

export type ValidationResult<T> =
  | { readonly ok: true; readonly value: T }
  | { readonly ok: false; readonly issues: readonly ValidationIssue[] };

const operationTypes = new Set<OperationType>([
  "polish",
  "continue_scene",
  "compress",
  "expand",
  "critique",
]);

const outputKinds = new Set<OutputKind>(["patch_proposal", "insert_proposal", "findings"]);
const granularities = new Set<DiffGranularity>(["paragraph", "sentence", "token"]);
const proposalStatuses = new Set<PatchProposalStatus>(["review", "accepted", "rejected", "conflicted"]);

function isRecord(value: unknown): value is Record<string, unknown> {
  return typeof value === "object" && value !== null && !Array.isArray(value);
}

function nonEmptyString(value: unknown): boolean {
  return typeof value === "string" && value.trim().length > 0;
}

export function validateOperationIntent(input: unknown): ValidationResult<OperationIntent> {
  const issues: ValidationIssue[] = [];
  if (!isRecord(input)) {
    return { ok: false, issues: [{ path: "$", message: "must be an object" }] };
  }

  if (input.schemaVersion !== 1) issues.push({ path: "$.schemaVersion", message: "must equal 1" });
  for (const field of ["id", "projectId", "baseCommitId", "createdAt"] as const) {
    if (!nonEmptyString(input[field])) issues.push({ path: `$.${field}`, message: "must be a non-empty string" });
  }
  if (!operationTypes.has(input.type as OperationType)) {
    issues.push({ path: "$.type", message: "is not a supported operation" });
  }
  if (!["low", "medium", "high"].includes(String(input.strength))) {
    issues.push({ path: "$.strength", message: "must be low, medium or high" });
  }

  if (!isRecord(input.target)) {
    issues.push({ path: "$.target", message: "must be an object" });
  } else {
    for (const field of ["documentId", "blockId", "baseHash"] as const) {
      if (!nonEmptyString(input.target[field])) issues.push({ path: `$.target.${field}`, message: "must be a non-empty string" });
    }
    if (!Number.isInteger(input.target.baseRevision) || Number(input.target.baseRevision) < 0) {
      issues.push({ path: "$.target.baseRevision", message: "must be a non-negative integer" });
    }
    for (const side of ["from", "to"] as const) {
      const anchor = input.target[side];
      if (!isRecord(anchor) || !nonEmptyString(anchor.blockId) || !Number.isInteger(anchor.offset) || Number(anchor.offset) < 0) {
        issues.push({ path: `$.target.${side}`, message: "must be a valid non-negative text anchor" });
      }
    }
  }

  if (!Array.isArray(input.constraints)) {
    issues.push({ path: "$.constraints", message: "must be an array" });
  }

  if (!isRecord(input.output)) {
    issues.push({ path: "$.output", message: "must be an object" });
  } else {
    if (!outputKinds.has(input.output.kind as OutputKind)) {
      issues.push({ path: "$.output.kind", message: "is not supported" });
    }
    if (!Number.isInteger(input.output.maxTokens) || Number(input.output.maxTokens) <= 0) {
      issues.push({ path: "$.output.maxTokens", message: "must be a positive integer" });
    }
  }

  return issues.length ? { ok: false, issues } : { ok: true, value: input as unknown as OperationIntent };
}

export function validatePatchProposal(input: unknown): ValidationResult<PatchProposal> {
  const issues: ValidationIssue[] = [];
  if (!isRecord(input)) {
    return { ok: false, issues: [{ path: "$", message: "must be an object" }] };
  }

  if (input.schemaVersion !== 2) issues.push({ path: "$.schemaVersion", message: "must equal 2" });
  for (const field of ["id", "operationRunId", "baseCommitId", "createdAt"] as const) {
    if (!nonEmptyString(input[field])) issues.push({ path: `$.${field}`, message: "must be a non-empty string" });
  }
  if (typeof input.proposalHash !== "string" || !/^sha256:[a-f0-9]{64}$/.test(input.proposalHash)) {
    issues.push({ path: "$.proposalHash", message: "must be a sha256 digest" });
  }
  if (!proposalStatuses.has(input.status as PatchProposalStatus)) {
    issues.push({ path: "$.status", message: "is not a supported proposal status" });
  }
  if (!Array.isArray(input.warnings) || input.warnings.some((warning) => typeof warning !== "string")) {
    issues.push({ path: "$.warnings", message: "must be an array of strings" });
  }

  const target = validateTarget(input.target, "$.target", issues);
  const targetBlockId = target?.blockId;
  if (!Array.isArray(input.hunks) || input.hunks.length === 0) {
    issues.push({ path: "$.hunks", message: "must be a non-empty array" });
  } else {
    const ids = new Set<string>();
    let previousEnd = -1;
    let previousInsertionOffset: number | undefined;
    for (const [index, value] of input.hunks.entries()) {
      const path = `$.hunks[${index}]`;
      if (!isRecord(value)) {
        issues.push({ path, message: "must be an object" });
        continue;
      }
      if (!nonEmptyString(value.id)) {
        issues.push({ path: `${path}.id`, message: "must be a non-empty string" });
      } else if (ids.has(String(value.id))) {
        issues.push({ path: `${path}.id`, message: "must be unique" });
      } else {
        ids.add(String(value.id));
      }
      const from = validateAnchor(value.from, `${path}.from`, issues);
      const to = validateAnchor(value.to, `${path}.to`, issues);
      if (from && to) {
        if (from.blockId !== to.blockId || (targetBlockId && from.blockId !== targetBlockId)) {
          issues.push({ path, message: "anchors must reference the proposal target block" });
        }
        if (from.offset > to.offset) {
          issues.push({ path, message: "from offset must not exceed to offset" });
        }
        if (from.offset < previousEnd) {
          issues.push({ path, message: "hunks must be sorted and non-overlapping" });
        }
        if (
          from.offset === to.offset
          && previousInsertionOffset === from.offset
        ) {
          issues.push({ path, message: "multiple insertions at the same offset are ambiguous" });
        }
        previousInsertionOffset = from.offset === to.offset ? from.offset : undefined;
        if (target && (from.offset < target.from || to.offset > target.to)) {
          issues.push({ path, message: "hunk must stay within the proposal target range" });
        }
        previousEnd = Math.max(previousEnd, to.offset);
        if (typeof value.original === "string" && value.original.length !== to.offset - from.offset) {
          issues.push({ path: `${path}.original`, message: "UTF-16 length must match the hunk range" });
        }
      }
      if (typeof value.original !== "string") {
        issues.push({ path: `${path}.original`, message: "must be a string" });
      }
      if (typeof value.replacement !== "string") {
        issues.push({ path: `${path}.replacement`, message: "must be a string" });
      }
      if (!granularities.has(value.granularity as DiffGranularity)) {
        issues.push({ path: `${path}.granularity`, message: "is not supported" });
      }
      if (value.atomicGroup !== undefined && !nonEmptyString(value.atomicGroup)) {
        issues.push({ path: `${path}.atomicGroup`, message: "must be a non-empty string" });
      }
    }
  }

  return issues.length ? { ok: false, issues } : { ok: true, value: input as unknown as PatchProposal };
}

function validateTarget(
  value: unknown,
  path: string,
  issues: ValidationIssue[],
): { blockId: string; from: number; to: number } | undefined {
  if (!isRecord(value)) {
    issues.push({ path, message: "must be an object" });
    return undefined;
  }
  for (const field of ["documentId", "blockId", "baseHash"] as const) {
    if (!nonEmptyString(value[field])) issues.push({ path: `${path}.${field}`, message: "must be a non-empty string" });
  }
  if (!Number.isInteger(value.baseRevision) || Number(value.baseRevision) < 0) {
    issues.push({ path: `${path}.baseRevision`, message: "must be a non-negative integer" });
  }
  const from = validateAnchor(value.from, `${path}.from`, issues);
  const to = validateAnchor(value.to, `${path}.to`, issues);
  if (from && to) {
    if (
      from.blockId !== to.blockId
      || (typeof value.blockId === "string" && from.blockId !== value.blockId)
    ) {
      issues.push({ path, message: "target anchors must reference the target block" });
    }
    if (from.offset > to.offset) {
      issues.push({ path, message: "target start must not exceed target end" });
    }
  }
  return typeof value.blockId === "string" && from && to
    ? { blockId: value.blockId, from: from.offset, to: to.offset }
    : undefined;
}

function validateAnchor(
  value: unknown,
  path: string,
  issues: ValidationIssue[],
): { blockId: string; offset: number } | undefined {
  if (
    !isRecord(value)
    || !nonEmptyString(value.blockId)
    || !Number.isInteger(value.offset)
    || Number(value.offset) < 0
  ) {
    issues.push({ path, message: "must be a valid non-negative text anchor" });
    return undefined;
  }
  return { blockId: String(value.blockId), offset: Number(value.offset) };
}
