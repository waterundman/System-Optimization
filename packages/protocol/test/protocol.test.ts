import assert from "node:assert/strict";
import test from "node:test";
import { validateOperationIntent } from "../src/index.ts";

const valid = {
  schemaVersion: 1,
  id: "intent-1",
  projectId: "project-1",
  baseCommitId: "commit-1",
  type: "polish",
  strength: "low",
  target: {
    documentId: "document-1",
    blockId: "block-1",
    baseRevision: 2,
    baseHash: "sha256:base",
    from: { blockId: "block-1", offset: 0 },
    to: { blockId: "block-1", offset: 4 },
  },
  constraints: [],
  output: { kind: "patch_proposal", maxTokens: 500 },
  createdAt: "2026-07-14T00:00:00.000Z",
};

test("validates a supported OperationIntent", () => {
  const result = validateOperationIntent(valid);
  assert.equal(result.ok, true);
});

test("rejects an invalid target and output budget", () => {
  const result = validateOperationIntent({
    ...valid,
    target: { ...valid.target, baseRevision: -1 },
    output: { ...valid.output, maxTokens: 0 },
  });
  assert.equal(result.ok, false);
  if (!result.ok) {
    assert.deepEqual(
      result.issues.map((issue) => issue.path),
      ["$.target.baseRevision", "$.output.maxTokens"],
    );
  }
});

