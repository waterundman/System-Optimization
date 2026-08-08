import { readdir, readFile } from "node:fs/promises";
import { dirname, resolve } from "node:path";
import { fileURLToPath } from "node:url";

const repository = resolve(dirname(fileURLToPath(import.meta.url)), "..");
const root = resolve(repository, "packages/protocol/schemas");
const protocolSrc = resolve(repository, "packages/protocol/src");
const files = (await readdir(root)).filter((name) => name.endsWith(".schema.json"));

if (files.length < 4) {
  throw new Error(`Expected at least four protocol schemas, found ${files.length}`);
}

for (const file of files) {
  const schema = JSON.parse(await readFile(resolve(root, file), "utf8"));
  if (schema.$schema !== "https://json-schema.org/draft/2020-12/schema") {
    throw new Error(`${file}: unsupported or missing $schema`);
  }
  if (!schema.$id || !schema.title || schema.type !== "object") {
    throw new Error(`${file}: $id, title and object type are required`);
  }
}

// Schema <-> TS type drift guard (no runtime JSON-Schema engine, by design).
// Each entry lists the exact top-level `properties` keys and `required` fields
// the TS type in packages/protocol/src/types.ts declares for the same
// document. A mismatch here means one of the two sources of truth drifted.
// `note` documents intentional exemptions.
const expectations = [
  {
    file: "context-packet.schema.json",
    tsType: "ContextPacket",
    fields: ["schemaVersion", "id", "operationIntentId", "projectId", "baseCommitId", "providerLocality", "items", "exclusions", "budget", "compilerVersion", "operationProfileVersion", "compiledAt", "packetHash"],
    required: ["schemaVersion", "id", "operationIntentId", "projectId", "baseCommitId", "providerLocality", "items", "exclusions", "budget", "compilerVersion", "operationProfileVersion", "compiledAt", "packetHash"],
    note: "",
  },
  {
    file: "event-envelope.schema.json",
    tsType: "EventEnvelope",
    fields: ["schemaVersion", "id", "type", "occurredAt", "traceId", "projectId", "operationRunId", "sequence", "payload"],
    required: ["schemaVersion", "id", "type", "occurredAt", "traceId", "payload"],
    note: "projectId / operationRunId / sequence are optional in the TS type.",
  },
  {
    file: "model-provider-configuration.schema.json",
    tsType: "ModelProviderConfiguration",
    fields: ["schemaVersion", "id", "providerId", "enabled", "defaultModel", "credentialRef", "qwen", "openaiCompatible", "defaultTimeoutMs", "maxRequestBytes", "updatedAt"],
    required: ["schemaVersion", "id", "providerId", "enabled", "defaultModel", "credentialRef", "defaultTimeoutMs", "maxRequestBytes", "updatedAt"],
    note: "qwen / openaiCompatible are optional in the TS type.",
  },
  {
    file: "operation-intent.schema.json",
    tsType: "OperationIntent",
    fields: ["schemaVersion", "id", "projectId", "baseCommitId", "type", "strength", "target", "userInstruction", "constraints", "output", "createdAt"],
    required: ["schemaVersion", "id", "projectId", "baseCommitId", "type", "strength", "target", "constraints", "output", "createdAt"],
    note: "userInstruction is optional in the TS type.",
  },
  {
    file: "operation-persistence-bundle.schema.json",
    tsType: "OperationPersistenceBundleV1",
    fields: ["schemaVersion", "run", "contextPacket", "lifecycleEvents", "attempts", "artifact"],
    required: ["schemaVersion", "run", "lifecycleEvents"],
    note: "contextPacket / attempts / artifact are optional in the TS type; conditional required[] rules are enforced by allOf.",
  },
  {
    file: "patch-proposal.schema.json",
    tsType: "PatchProposal",
    fields: ["schemaVersion", "id", "operationRunId", "baseCommitId", "target", "hunks", "summary", "warnings", "status", "createdAt", "proposalHash"],
    required: ["schemaVersion", "id", "operationRunId", "baseCommitId", "target", "hunks", "warnings", "status", "createdAt", "proposalHash"],
    note: "summary is optional in the TS type.",
  },
  {
    file: "persist-review-event-command.schema.json",
    tsType: "PersistReviewEventCommandV1",
    fields: ["schemaVersion", "id", "proposalId", "expectedRevision", "expectedStatus", "kind", "nextStatus", "hunkId", "decision", "payload", "occurredAt"],
    required: ["schemaVersion", "id", "proposalId", "expectedRevision", "expectedStatus", "kind", "nextStatus", "occurredAt"],
    note: "hunkId / decision / payload are optional in the TS type.",
  },
  {
    file: "project-snapshot.schema.json",
    tsType: "Rust-host concept (no TS type)",
    fields: ["schemaVersion", "projectId", "commitId", "rootHash", "documents", "blocks"],
    required: ["schemaVersion", "projectId", "commitId", "rootHash", "documents", "blocks"],
    note: "Exemption: ProjectSnapshot mirrors crates/optimizer-store snapshots; its document/block item shapes are mirrored by packages/protocol/src/storage.ts for Rust reference only.",
  },
];

const normalized = (items) => [...items].sort();
function assertSameList(file, label, actual, expected) {
  const a = normalized(actual);
  const b = normalized(expected);
  if (a.length !== b.length || a.some((value, index) => value !== b[index])) {
    throw new Error(
      `${file}: ${label} mismatch with the declared TS type.\n` +
        `  schema:   ${a.join(", ")}\n` +
        `  expected: ${b.join(", ")}`,
    );
  }
}

for (const expectation of expectations) {
  const schema = JSON.parse(await readFile(resolve(root, expectation.file), "utf8"));
  assertSameList(expectation.file, "properties keys", Object.keys(schema.properties ?? {}), expectation.fields);
  assertSameList(expectation.file, "required fields", schema.required ?? [], expectation.required);
}

// OperationIntent constraints must match the validator in validation.ts:
// severity enum, rule length cap, max item count, userInstruction cap.
const validationSource = await readFile(resolve(protocolSrc, "validation.ts"), "utf8");
const operationIntentSchema = JSON.parse(
  await readFile(resolve(root, "operation-intent.schema.json"), "utf8"),
);

const constraintItem = operationIntentSchema.$defs?.constraints
  ?? operationIntentSchema.properties?.constraints?.items;
if (!constraintItem) {
  throw new Error("operation-intent.schema.json: constraints items definition is missing");
}

const schemaSeverities = constraintItem.properties?.severity?.enum ?? [];
const validatorSeverities = extractSet(validationSource, "constraintSeverities");
if (JSON.stringify([...schemaSeverities].sort()) !== JSON.stringify([...validatorSeverities].sort())) {
  throw new Error(
    `operation-intent: constraints severity enum drift.\n` +
      `  schema:    ${schemaSeverities.join(", ")}\n` +
      `  validator: ${validatorSeverities.join(", ")}`,
  );
}

const numericConstants = {
  maxConstraints: ["constraints", "maxItems"],
  maxConstraintRuleLength: ["constraints", "items", "properties", "rule", "maxLength"],
  maxUserInstructionLength: ["userInstruction", "maxLength"],
};
for (const [validatorConstant, segments] of Object.entries(numericConstants)) {
  const schemaValue = numericSchemaValue(operationIntentSchema, segments);
  const validatorValue = extractNumericConstant(validationSource, validatorConstant);
  if (schemaValue === undefined || validatorValue === undefined) {
    throw new Error(
      `operation-intent: could not read "${segments.join(".")}" (schema=${schemaValue}) / "${validatorConstant}" (validator=${validatorValue})`,
    );
  }
  if (schemaValue !== validatorValue) {
    throw new Error(
      `operation-intent: ${segments.join(".")} drift.\n` +
        `  schema:    ${schemaValue}\n` +
        `  validator: ${validatorValue}`,
    );
  }
}

console.log(`Schema check passed: ${files.length} files (metadata + TS drift guard)`);

function extractSet(source, declarationName) {
  const match = source.match(new RegExp(`const ${declarationName}\\s*=\\s*new Set\\(\\[([^\\]]+)\\]\\)`));
  if (!match) return [];
  return [...match[1].matchAll(/["']([^"']+)["']/g)].map((item) => item[1]).sort();
}

function extractNumericConstant(source, name) {
  const match = source.match(new RegExp(`const ${name}\\s*=\\s*(\\d[\\d_]*)`));
  return match ? Number(match[1].replaceAll("_", "")) : undefined;
}

function numericSchemaValue(schema, segments) {
  let node = schema.properties;
  for (const key of segments) {
    node = node?.[key];
    if (node === undefined) return undefined;
  }
  return typeof node === "number" ? node : undefined;
}
