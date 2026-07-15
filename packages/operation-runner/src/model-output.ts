import type { OutputKind } from "../../protocol/src/index.ts";
import { OperationStageError } from "./errors.ts";
import type {
  OperationFinding,
  ParsedModelOutput,
} from "./types.ts";

const summaryLimit = 2_000;
const findingLimit = 200;
const findingMessageLimit = 5_000;

export function parseModelOutput(text: string, expectedKind: OutputKind): ParsedModelOutput {
  let value: unknown;
  try {
    value = JSON.parse(text);
  } catch {
    throw invalid("Model output must be one complete JSON object without Markdown fencing");
  }
  const root = record(value, "model output");
  exactFields(root, expectedKind === "findings"
    ? ["schemaVersion", "kind", "findings", "summary"]
    : ["schemaVersion", "kind", "replacementText", "summary"]);
  if (root.schemaVersion !== 1) throw invalid("schemaVersion must equal 1");
  const summary = optionalBoundedString(root.summary, "summary", summaryLimit);

  if (expectedKind === "findings") {
    if (root.kind !== "findings") throw invalid("Expected a findings output");
    if (!Array.isArray(root.findings) || root.findings.length > findingLimit) {
      throw invalid(`findings must be an array with at most ${findingLimit} items`);
    }
    const findings = root.findings.map(parseFinding);
    return { kind: "findings", findings, ...(summary !== undefined ? { summary } : {}) };
  }

  if (root.kind !== "replacement") throw invalid("Expected a replacement output");
  if (typeof root.replacementText !== "string") {
    throw invalid("replacementText must be a string");
  }
  return {
    kind: "replacement",
    replacementText: root.replacementText,
    ...(summary !== undefined ? { summary } : {}),
  };
}

function parseFinding(value: unknown, index: number): OperationFinding {
  const finding = record(value, `findings[${index}]`);
  exactFields(finding, ["severity", "message", "sourceRef"]);
  if (!(["info", "warning", "error"] as const).includes(finding.severity as never)) {
    throw invalid(`findings[${index}].severity is invalid`);
  }
  const message = boundedString(finding.message, `findings[${index}].message`, findingMessageLimit);
  const sourceRef = optionalBoundedString(finding.sourceRef, `findings[${index}].sourceRef`, 1_000);
  return {
    severity: finding.severity as OperationFinding["severity"],
    message,
    ...(sourceRef !== undefined ? { sourceRef } : {}),
  };
}

function record(value: unknown, name: string): Readonly<Record<string, unknown>> {
  if (!value || typeof value !== "object" || Array.isArray(value)) {
    throw invalid(`${name} must be an object`);
  }
  return value as Readonly<Record<string, unknown>>;
}

function exactFields(value: Readonly<Record<string, unknown>>, allowed: readonly string[]): void {
  const allowedSet = new Set(allowed);
  const unknown = Object.keys(value).filter((field) => !allowedSet.has(field));
  if (unknown.length > 0) throw invalid(`Unknown model output field: ${unknown[0]}`);
}

function boundedString(value: unknown, name: string, limit: number): string {
  if (typeof value !== "string" || value.length === 0 || value.length > limit) {
    throw invalid(`${name} must be a non-empty string with at most ${limit} UTF-16 code units`);
  }
  return value;
}

function optionalBoundedString(value: unknown, name: string, limit: number): string | undefined {
  if (value === undefined) return undefined;
  return boundedString(value, name, limit);
}

function invalid(message: string): OperationStageError {
  return new OperationStageError("MODEL_OUTPUT_INVALID", message);
}
