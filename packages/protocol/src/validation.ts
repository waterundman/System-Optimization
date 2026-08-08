import type {
  DiffGranularity,
  ModelProviderConfiguration,
  ModelProviderId,
  OperationIntent,
  OperationType,
  OutputKind,
  PatchProposal,
  PatchProposalStatus,
  QwenDeploymentRegion,
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

const constraintSeverities = new Set(["hard", "soft"]);
const maxConstraints = 256;
const maxConstraintRuleLength = 10_000;
const maxUserInstructionLength = 10_000;

const outputKinds = new Set<OutputKind>(["patch_proposal", "insert_proposal", "findings"]);
const granularities = new Set<DiffGranularity>(["paragraph", "sentence", "token"]);
const proposalStatuses = new Set<PatchProposalStatus>(["review", "accepted", "rejected", "conflicted"]);
const modelProviderIds = new Set<ModelProviderId>([
  "deepseek",
  "qwen",
  "kimi",
  "minimax",
  "ollama",
  "openai_compatible",
]);
const qwenRegions = new Set<QwenDeploymentRegion>([
  "china",
  "singapore",
  "us",
  "germany",
  "japan",
]);

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
  } else {
    if (input.constraints.length > maxConstraints) {
      issues.push({ path: "$.constraints", message: `must contain at most ${maxConstraints} constraints` });
    }
    for (const [index, value] of input.constraints.entries()) {
      const path = `$.constraints[${index}]`;
      if (!isRecord(value)) {
        issues.push({ path, message: "must be an object" });
        continue;
      }
      for (const field of Object.keys(value)) {
        if (field !== "severity" && field !== "rule" && field !== "sourceRef") {
          issues.push({ path: `${path}.${field}`, message: "is not allowed" });
        }
      }
      if (!constraintSeverities.has(String(value.severity))) {
        issues.push({ path: `${path}.severity`, message: "must be hard or soft" });
      }
      if (!nonEmptyString(value.rule)) {
        issues.push({ path: `${path}.rule`, message: "must be a non-empty string" });
      } else if (String(value.rule).length > maxConstraintRuleLength) {
        issues.push({ path: `${path}.rule`, message: `must be at most ${maxConstraintRuleLength} characters` });
      }
      if (value.sourceRef !== undefined && !nonEmptyString(value.sourceRef)) {
        issues.push({ path: `${path}.sourceRef`, message: "must be a non-empty string" });
      }
    }
  }

  if (
    input.userInstruction !== undefined
    && (
      typeof input.userInstruction !== "string"
      || input.userInstruction.length > maxUserInstructionLength
    )
  ) {
    issues.push({
      path: "$.userInstruction",
      message: `must be a string of at most ${maxUserInstructionLength} characters`,
    });
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

export function validateModelProviderConfiguration(
  input: unknown,
): ValidationResult<ModelProviderConfiguration> {
  const issues: ValidationIssue[] = [];
  if (!isRecord(input)) {
    return { ok: false, issues: [{ path: "$", message: "must be an object" }] };
  }
  const allowedFields = new Set([
    "schemaVersion",
    "id",
    "providerId",
    "enabled",
    "defaultModel",
    "credentialRef",
    "qwen",
    "openaiCompatible",
    "defaultTimeoutMs",
    "maxRequestBytes",
    "updatedAt",
  ]);
  for (const field of Object.keys(input)) {
    if (!allowedFields.has(field)) {
      issues.push({ path: `$.${field}`, message: "is not allowed; secrets must not be persisted" });
    }
  }
  if (input.schemaVersion !== 1) {
    issues.push({ path: "$.schemaVersion", message: "must equal 1" });
  }
  for (const field of ["id", "defaultModel", "updatedAt"] as const) {
    if (!nonEmptyString(input[field])) {
      issues.push({ path: `$.${field}`, message: "must be a non-empty string" });
    }
  }
  if (!modelProviderIds.has(input.providerId as ModelProviderId)) {
    issues.push({ path: "$.providerId", message: "is not a supported provider" });
  }
  if (typeof input.enabled !== "boolean") {
    issues.push({ path: "$.enabled", message: "must be a boolean" });
  }
  if (
    typeof input.credentialRef !== "string"
    || !/^secret:\/\/[A-Za-z0-9._~/-]+$/.test(input.credentialRef)
  ) {
    issues.push({ path: "$.credentialRef", message: "must be an opaque secret:// reference" });
  }
  validateBoundedInteger(input.defaultTimeoutMs, "$.defaultTimeoutMs", 1, 3_600_000, issues);
  validateBoundedInteger(input.maxRequestBytes, "$.maxRequestBytes", 1, 67_108_864, issues);

  if (input.qwen !== undefined) {
    if (input.providerId !== "qwen") {
      issues.push({ path: "$.qwen", message: "is only valid for the qwen provider" });
    } else if (!isRecord(input.qwen)) {
      issues.push({ path: "$.qwen", message: "must be an object" });
    } else {
      if (!qwenRegions.has(input.qwen.region as QwenDeploymentRegion)) {
        issues.push({ path: "$.qwen.region", message: "is not a supported Qwen region" });
      }
      if (
        input.qwen.workspaceId !== undefined
        && (
          typeof input.qwen.workspaceId !== "string"
          || !/^[A-Za-z0-9_-]+$/.test(input.qwen.workspaceId)
        )
      ) {
        issues.push({ path: "$.qwen.workspaceId", message: "contains unsupported characters" });
      }
      for (const field of Object.keys(input.qwen)) {
        if (field !== "region" && field !== "workspaceId") {
          issues.push({ path: `$.qwen.${field}`, message: "is not allowed" });
        }
      }
    }
  }
  if (input.openaiCompatible !== undefined) {
    if (input.providerId !== "openai_compatible") {
      issues.push({ path: "$.openaiCompatible", message: "is only valid for the openai_compatible provider" });
    } else if (!isRecord(input.openaiCompatible)) {
      issues.push({ path: "$.openaiCompatible", message: "must be an object" });
    } else {
      const compatible = input.openaiCompatible;
      if (
        typeof compatible.endpointId !== "string"
        || !/^endpoint-[a-f0-9]{32}$/.test(compatible.endpointId)
      ) {
        issues.push({ path: "$.openaiCompatible.endpointId", message: "must be a Host-issued endpoint ID" });
      } else {
        if (input.id !== `provider-openai-compatible-${compatible.endpointId}`) {
          issues.push({
            path: "$.id",
            message: "must be derived from the Host-issued endpoint ID",
          });
        }
        if (
          input.credentialRef
          !== `secret://providers/openai-compatible/${compatible.endpointId}`
        ) {
          issues.push({
            path: "$.credentialRef",
            message: "must match the Host-issued endpoint credential slot",
          });
        }
      }
      if (compatible.endpointRevision !== 1) {
        issues.push({ path: "$.openaiCompatible.endpointRevision", message: "must equal 1" });
      }
      for (const field of ["jsonObject", "streamUsage"] as const) {
        if (typeof compatible[field] !== "boolean") {
          issues.push({ path: `$.openaiCompatible.${field}`, message: "must be a boolean" });
        }
      }
      if (!new Set(["max_tokens", "max_completion_tokens"]).has(
        compatible.maxOutputTokenField as string,
      )) {
        issues.push({
          path: "$.openaiCompatible.maxOutputTokenField",
          message: "must be max_tokens or max_completion_tokens",
        });
      }
      const allowed = new Set([
        "endpointId",
        "endpointRevision",
        "jsonObject",
        "streamUsage",
        "maxOutputTokenField",
      ]);
      for (const field of Object.keys(compatible)) {
        if (!allowed.has(field)) {
          issues.push({ path: `$.openaiCompatible.${field}`, message: "is not allowed" });
        }
      }
    }
  } else if (input.providerId === "openai_compatible") {
    issues.push({
      path: "$.openaiCompatible",
      message: "is required for the openai_compatible provider",
    });
  }
  return issues.length
    ? { ok: false, issues }
    : { ok: true, value: input as unknown as ModelProviderConfiguration };
}

function validateBoundedInteger(
  value: unknown,
  path: string,
  minimum: number,
  maximum: number,
  issues: ValidationIssue[],
): void {
  if (!Number.isInteger(value) || Number(value) < minimum || Number(value) > maximum) {
    issues.push({ path, message: `must be an integer between ${minimum} and ${maximum}` });
  }
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
