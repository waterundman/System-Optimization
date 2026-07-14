import type { OperationIntent, OperationType, OutputKind } from "./types.ts";

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

