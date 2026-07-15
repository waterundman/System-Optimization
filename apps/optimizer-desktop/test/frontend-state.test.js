import assert from "node:assert/strict";
import test from "node:test";
import {
  applySaveResponse,
  blocksForDocument,
  buildSaveBlockRequest,
  countVisibleCharacters,
  normalizeHostError,
  selectInitialDocument,
} from "../src/frontend-state.js";

function workspace() {
  return {
    schemaVersion: 1,
    projectId: "project-1",
    mainBranchId: "branch-1",
    headCommitId: "commit-1",
    revision: 2,
    documents: [
      { id: "document-b", parentId: null, kind: "chapter", title: "第二章", orderKey: "b", revision: 0 },
      { id: "document-a", parentId: null, kind: "chapter", title: "第一章", orderKey: "a", revision: 4 },
    ],
    blocks: [
      {
        id: "block-2",
        documentId: "document-a",
        kind: "paragraph",
        orderKey: "b",
        content: { type: "paragraph" },
        plainText: "第二段",
        contentHash: "sha256:block-2",
        revision: 1,
        locked: false,
      },
      {
        id: "block-1",
        documentId: "document-a",
        kind: "paragraph",
        orderKey: "a",
        content: { type: "paragraph" },
        plainText: "第一段",
        contentHash: "sha256:block-1",
        revision: 3,
        locked: false,
      },
    ],
  };
}

test("selects documents and blocks by stable order keys", () => {
  const source = workspace();
  assert.equal(selectInitialDocument(source), "document-a");
  assert.equal(selectInitialDocument(source, "document-b"), "document-b");
  assert.deepEqual(blocksForDocument(source, "document-a").map((block) => block.id), ["block-1", "block-2"]);
});

test("builds a versioned save request without accepting client commit metadata", () => {
  const block = workspace().blocks[0];
  assert.deepEqual(buildSaveBlockRequest(block, "修订正文"), {
    schemaVersion: 1,
    blockId: "block-2",
    expectedRevision: 1,
    expectedHash: "sha256:block-2",
    content: { type: "paragraph", content: [{ type: "text", text: "修订正文" }] },
    plainText: "修订正文",
  });
});

test("applies an authoritative save response and advances its document baseline", () => {
  const source = workspace();
  const savedBlock = { ...source.blocks[0], revision: 2, plainText: "已保存", contentHash: "sha256:saved" };
  const next = applySaveResponse(source, {
    headCommitId: "commit-2",
    projectRevision: 3,
    block: savedBlock,
  });
  assert.equal(next.headCommitId, "commit-2");
  assert.equal(next.revision, 3);
  assert.equal(next.documents.find((item) => item.id === "document-a")?.revision, 5);
  assert.equal(next.blocks.find((item) => item.id === savedBlock.id)?.plainText, "已保存");
  assert.equal(source.headCommitId, "commit-1");
});

test("normalizes host failures and counts visible writing characters", () => {
  assert.deepEqual(normalizeHostError({ code: "CONFLICT", message: "changed" }), {
    code: "CONFLICT",
    message: "changed",
  });
  assert.deepEqual(normalizeHostError("offline"), { code: "HOST_ERROR", message: "offline" });
  assert.equal(countVisibleCharacters(workspace()), 6);
});
