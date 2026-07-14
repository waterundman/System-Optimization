import assert from "node:assert/strict";
import { createHash } from "node:crypto";
import test from "node:test";
import type {
  BlockId,
  CommitId,
  DocumentId,
  OperationRunId,
  PatchProposal,
  PatchProposalId,
} from "../../protocol/src/index.ts";
import {
  compileEditorTransaction,
  type EditorBlockSnapshot,
  type EditorDocumentSnapshot,
} from "../../editor-bridge/src/index.ts";
import {
  PatchEngineError,
  compilePatchProposal,
  compileReviewTransaction,
  createPatchReview,
  decidePatchHunk,
  diffText,
  finalizePatchReview,
  segmentText,
  verifyProposalHash,
  type PatchHasher,
  type PatchReviewSession,
} from "../src/index.ts";

const documentId = "document-1" as DocumentId;
const blockId = "block-1" as BlockId;
const prefix = "前文：";
const selected = "天气很好。我们出发。";
const suffix = "尾声";
const replacement = "天气晴朗。我们立即出发。";

const hasher: PatchHasher = {
  id: "test:sha256",
  async sha256(value: string) {
    return `sha256:${createHash("sha256").update(value, "utf8").digest("hex")}`;
  },
};

function block(overrides: Partial<EditorBlockSnapshot> = {}): EditorBlockSnapshot {
  return {
    id: blockId,
    documentId,
    kind: "paragraph",
    orderKey: "a",
    content: { type: "paragraph", text: prefix + selected + suffix },
    plainText: prefix + selected + suffix,
    contentHash: "sha256:block-base",
    revision: 5,
    locked: false,
    ...overrides,
  };
}

function document(target = block()): EditorDocumentSnapshot {
  return { documentId, revision: 11, blocks: [target] };
}

async function proposal(overrides: { atomic?: boolean; replacementText?: string } = {}) {
  const target = block();
  return compilePatchProposal({
    id: "proposal-1" as PatchProposalId,
    operationRunId: "run-1" as OperationRunId,
    baseCommitId: "commit-1" as CommitId,
    target: {
      documentId,
      blockId,
      baseRevision: target.revision,
      baseHash: target.contentHash,
      from: { blockId, offset: prefix.length },
      to: { blockId, offset: prefix.length + selected.length },
    },
    block: target,
    replacementText: overrides.replacementText ?? replacement,
    createdAt: "2026-07-14T08:00:00.000Z",
    atomic: overrides.atomic,
  }, hasher);
}

function decideAll(
  value: PatchProposal,
  session: PatchReviewSession,
  decision: "accepted" | "rejected",
): PatchReviewSession {
  let current = session;
  for (const hunk of value.hunks) {
    if (current.decisions[hunk.id] === decision) continue;
    current = decidePatchHunk(value, current, {
      hunkId: hunk.id,
      decision,
      expectedRevision: current.revision,
    });
  }
  return current;
}

test("segments Chinese text deterministically and keeps a ZWJ emoji intact", () => {
  assert.deepEqual(
    segmentText("甲。乙！", "sentence").map((unit) => unit.text),
    ["甲。", "乙！"],
  );
  assert.deepEqual(
    segmentText("A股上涨👨‍👩‍👧‍👦", "token").map((unit) => unit.text),
    ["A", "股", "上", "涨", "👨‍👩‍👧‍👦"],
  );
});

test("builds a layered Chinese diff that reconstructs the replacement exactly", () => {
  const diff = diffText(selected, replacement);
  assert.equal(diff.changes.length, 2);
  assert.equal(diff.changes[0]?.original, "很好");
  assert.equal(diff.changes[0]?.replacement, "晴朗");
  assert.equal(diff.changes[1]?.original, "");
  assert.equal(diff.changes[1]?.replacement, "立即");
  assert.equal(applyChanges(selected, diff.changes), replacement);
  assert.equal(diff.warnings.length, 0);
});

test("uses an explicit safe fallback when the LCS complexity budget is exceeded", () => {
  const diff = diffText("甲乙丙丁", "戊己庚辛", { maxMatrixCells: 1 });
  assert.deepEqual(diff.warnings, ["DIFF_COMPLEXITY_FALLBACK"]);
  assert.equal(applyChanges("甲乙丙丁", diff.changes), "戊己庚辛");
});

test("reconstructs both sides across deterministic mixed-language cases", () => {
  const alphabet = ["甲", "乙", "。", "A", " ", "😀"];
  let state = 0x5eed1234;
  const next = () => {
    state = (Math.imul(state, 1_664_525) + 1_013_904_223) >>> 0;
    return state;
  };
  const makeText = () => {
    const length = next() % 10;
    let value = "";
    for (let index = 0; index < length; index += 1) {
      value += alphabet[next() % alphabet.length];
    }
    return value;
  };

  for (let index = 0; index < 400; index += 1) {
    const original = makeText();
    const revised = makeText();
    const diff = diffText(original, revised);
    assert.equal(
      diff.segments
        .filter((segment) => segment.kind !== "insert")
        .map((segment) => segment.text)
        .join(""),
      original,
    );
    assert.equal(
      diff.segments
        .filter((segment) => segment.kind !== "delete")
        .map((segment) => segment.text)
        .join(""),
      revised,
    );
    assert.equal(applyChanges(original, diff.changes), revised);
  }
});

test("compiles absolute review hunks and binds them with a proposal hash", async () => {
  const value = await proposal();
  assert.equal(value.hunks.length, 2);
  assert.equal(value.hunks[0]?.from.offset, prefix.length + 2);
  assert.equal(value.hunks[0]?.original, "很好");
  assert.equal(value.hunks[0]?.granularity, "token");
  assert.match(value.proposalHash, /^sha256:[a-f0-9]{64}$/);
  assert.equal(await verifyProposalHash(value, hasher), true);
});

test("an atomic decision propagates to every hunk", async () => {
  const value = await proposal({ atomic: true });
  let review = await createPatchReview(value, hasher);
  review = decidePatchHunk(value, review, {
    hunkId: value.hunks[0]?.id ?? "",
    decision: "accepted",
    expectedRevision: 0,
  });
  assert.equal(review.status, "ready");
  assert.deepEqual(new Set(Object.values(review.decisions)), new Set(["accepted"]));
});

test("partial acceptance compiles one editor transaction and preserves rejected text", async () => {
  const value = await proposal();
  let review = await createPatchReview(value, hasher);
  review = decidePatchHunk(value, review, {
    hunkId: value.hunks[0]?.id ?? "",
    decision: "accepted",
    expectedRevision: review.revision,
  });
  review = decidePatchHunk(value, review, {
    hunkId: value.hunks[1]?.id ?? "",
    decision: "rejected",
    expectedRevision: review.revision,
  });

  const compiled = await compileReviewTransaction({
    proposal: value,
    session: review,
    document: document(),
    transactionId: "editor-tx-1",
    hasher,
    contentAdapter: {
      replacePlainText(current, nextPlainText) {
        return { ...current.content, text: nextPlainText };
      },
    },
  });
  assert.equal(compiled.status, "ready_to_apply");
  if (compiled.status !== "ready_to_apply") return;
  assert.equal(compiled.nextPlainText, prefix + "天气晴朗。我们出发。" + suffix);

  const editorResult = await compileEditorTransaction(document(), compiled.transaction, hasher);
  assert.equal(editorResult.nextDocument.blocks[0]?.plainText, compiled.nextPlainText);
});

test("returns a conflict instead of relocating a proposal over changed content", async () => {
  const value = await proposal();
  const review = decideAll(value, await createPatchReview(value, hasher), "accepted");
  const result = await compileReviewTransaction({
    proposal: value,
    session: review,
    document: document(block({
      revision: 6,
      contentHash: "sha256:changed",
      plainText: prefix + "天气变了。我们出发。" + suffix,
    })),
    transactionId: "editor-tx-conflict",
    hasher,
    contentAdapter: { replacePlainText: (_current, text) => ({ text }) },
  });
  assert.equal(result.status, "conflicted");
  if (result.status === "conflicted") {
    assert.equal(result.conflicts[0]?.code, "TARGET_BLOCK_CHANGED");
  }
});

test("detects a changed hunk source even when a faulty adapter reuses revision metadata", async () => {
  const value = await proposal();
  const review = decideAll(value, await createPatchReview(value, hasher), "accepted");
  const altered = block({ plainText: prefix + "天气很坏。我们出发。" + suffix });
  const result = await compileReviewTransaction({
    proposal: value,
    session: review,
    document: document(altered),
    transactionId: "editor-tx-source-conflict",
    hasher,
    contentAdapter: { replacePlainText: (_current, text) => ({ text }) },
  });
  assert.equal(result.status, "conflicted");
  if (result.status === "conflicted") {
    assert.equal(result.conflicts[0]?.code, "HUNK_SOURCE_CHANGED");
  }
});

test("rejects a proposal if its content is changed after hashing", async () => {
  const value = await proposal();
  const tampered = { ...value, summary: "被篡改" };
  await assert.rejects(
    createPatchReview(tampered, hasher),
    (error: unknown) => error instanceof PatchEngineError && error.code === "PROPOSAL_INVALID",
  );
});

test("all rejected hunks produce no editor transaction", async () => {
  const value = await proposal();
  const review = decideAll(value, await createPatchReview(value, hasher), "rejected");
  const result = await compileReviewTransaction({
    proposal: value,
    session: review,
    document: document(),
    transactionId: "unused",
    hasher,
    contentAdapter: { replacePlainText: (_current, text) => ({ text }) },
  });
  assert.equal(result.status, "rejected");
  const finalized = finalizePatchReview(review, "rejected", review.revision);
  assert.equal(finalized.status, "rejected");
});

test("rejects a no-op model response", async () => {
  await assert.rejects(
    proposal({ replacementText: selected }),
    (error: unknown) => error instanceof PatchEngineError && error.code === "NO_CHANGES",
  );
});

function applyChanges(
  source: string,
  changes: readonly {
    readonly from: number;
    readonly to: number;
    readonly replacement: string;
  }[],
): string {
  let result = source;
  for (const change of [...changes].sort((left, right) => right.from - left.from)) {
    result = result.slice(0, change.from) + change.replacement + result.slice(change.to);
  }
  return result;
}
