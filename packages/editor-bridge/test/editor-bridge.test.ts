import assert from "node:assert/strict";
import { createHash } from "node:crypto";
import test from "node:test";
import { stableStringify, type BlockId, type DocumentId } from "../../protocol/src/index.ts";
import {
  EditorBridgeError,
  compileEditorTransaction,
  fromTiptapSnapshot,
  selectionToOperationTarget,
  type EditorBlockSnapshot,
  type EditorDocumentSnapshot,
  type EditorHasher,
  type EditorTransaction,
} from "../src/index.ts";

const documentId = "document-1" as DocumentId;
const block1 = "block-1" as BlockId;
const block2 = "block-2" as BlockId;

const hasher: EditorHasher = {
  id: "test:sha256",
  async sha256(value: string) {
    return `sha256:${createHash("sha256").update(value, "utf8").digest("hex")}`;
  },
};

function block(overrides: Partial<EditorBlockSnapshot> = {}): EditorBlockSnapshot {
  return {
    id: block1,
    documentId,
    kind: "paragraph",
    orderKey: "a",
    content: { type: "paragraph", text: "你好😀世界" },
    plainText: "你好😀世界",
    contentHash: "sha256:base",
    revision: 3,
    locked: false,
    ...overrides,
  };
}

function document(blocks: readonly EditorBlockSnapshot[] = [block()]): EditorDocumentSnapshot {
  return { documentId, revision: 7, blocks };
}

function transaction(
  steps: EditorTransaction["steps"],
  overrides: Partial<EditorTransaction> = {},
): EditorTransaction {
  return {
    id: "editor-transaction-1",
    documentId,
    expectedDocumentRevision: 7,
    steps,
    ...overrides,
  };
}

function hasCode(code: string) {
  return (error: unknown) => error instanceof EditorBridgeError && error.code === code;
}

test("maps a reverse same-block UTF-16 selection to a stable operation target", () => {
  const result = selectionToOperationTarget(document(), {
    anchor: { blockId: block1, offset: 6 },
    head: { blockId: block1, offset: 2 },
    offsetEncoding: "utf16",
  });

  assert.equal(result.selectedText, "😀世界");
  assert.deepEqual(result.target.from, { blockId: block1, offset: 2, affinity: "after" });
  assert.deepEqual(result.target.to, { blockId: block1, offset: 6, affinity: "before" });
  assert.equal(result.target.baseRevision, 3);
  assert.equal(result.target.baseHash, "sha256:base");
});

test("rejects cross-block selections in the MVP", () => {
  assert.throws(
    () => selectionToOperationTarget(document(), {
      anchor: { blockId: block1, offset: 0 },
      head: { blockId: block2, offset: 0 },
      offsetEncoding: "utf16",
    }),
    hasCode("SELECTION_CROSSES_BLOCKS"),
  );
});

test("rejects a UTF-16 offset that splits a surrogate pair", () => {
  assert.throws(
    () => selectionToOperationTarget(document(), {
      anchor: { blockId: block1, offset: 3 },
      head: { blockId: block1, offset: 6 },
      offsetEncoding: "utf16",
    }),
    hasCode("SELECTION_OFFSET_INVALID"),
  );
});

test("compiles a replace step without mutating the source snapshot", async () => {
  const source = document();
  const compiled = await compileEditorTransaction(source, transaction([{
    type: "replace_block",
    blockId: block1,
    expectedRevision: 3,
    expectedHash: "sha256:base",
    nextContent: { type: "paragraph", text: "修订正文" },
    nextPlainText: "修订正文",
  }]), hasher);

  assert.equal(source.blocks[0]?.plainText, "你好😀世界");
  assert.equal(compiled.nextDocument.revision, 8);
  assert.equal(compiled.nextDocument.blocks[0]?.revision, 4);
  assert.equal(compiled.nextDocument.blocks[0]?.plainText, "修订正文");
  assert.match(compiled.nextDocument.blocks[0]?.contentHash ?? "", /^sha256:[a-f0-9]{64}$/);
  assert.equal(compiled.changes[0]?.operation, "update");
});

test("content hashes are canonical and independent from stable identity and order", async () => {
  const content = { type: "paragraph", attrs: { z: 2, a: 1 } };
  const first = await compileEditorTransaction(document(), transaction([{
    type: "replace_block",
    blockId: block1,
    expectedRevision: 3,
    expectedHash: "sha256:base",
    nextContent: content,
    nextPlainText: "same",
  }]), hasher);
  const other = block({ id: block2, orderKey: "z" });
  const second = await compileEditorTransaction(document([other]), transaction([{
    type: "replace_block",
    blockId: block2,
    expectedRevision: 3,
    expectedHash: "sha256:base",
    nextContent: { attrs: { a: 1, z: 2 }, type: "paragraph" },
    nextPlainText: "same",
  }]), hasher);

  assert.equal(first.nextDocument.blocks[0]?.contentHash, second.nextDocument.blocks[0]?.contentHash);
  assert.equal(stableStringify({ z: 2, a: 1 }), '{"a":1,"z":2}');
});

test("rejects stale, locked, empty and cross-document transactions", async () => {
  await assert.rejects(
    compileEditorTransaction(document(), transaction([{
      type: "delete_block",
      blockId: block1,
      expectedRevision: 2,
      expectedHash: "sha256:base",
    }]), hasher),
    hasCode("BLOCK_REVISION_CONFLICT"),
  );
  await assert.rejects(
    compileEditorTransaction(document([block({ locked: true })]), transaction([{
      type: "delete_block",
      blockId: block1,
      expectedRevision: 3,
      expectedHash: "sha256:base",
    }]), hasher),
    hasCode("BLOCK_LOCKED"),
  );
  await assert.rejects(
    compileEditorTransaction(document(), transaction([]), hasher),
    hasCode("EMPTY_TRANSACTION"),
  );
  await assert.rejects(
    compileEditorTransaction(document(), transaction([{
      type: "insert_block",
      block: {
        id: block2,
        documentId: "document-other" as DocumentId,
        kind: "paragraph",
        orderKey: "b",
        content: {},
        plainText: "",
        locked: false,
      },
    }]), hasher),
    hasCode("DOCUMENT_ID_CONFLICT"),
  );
});

test("rejects duplicate order keys and a block touched twice", async () => {
  await assert.rejects(
    compileEditorTransaction(document(), transaction([{
      type: "insert_block",
      block: {
        id: block2,
        documentId,
        kind: "paragraph",
        orderKey: "a",
        content: {},
        plainText: "",
        locked: false,
      },
    }]), hasher),
    hasCode("ORDER_KEY_DUPLICATE"),
  );
  await assert.rejects(
    compileEditorTransaction(document(), transaction([
      {
        type: "move_block",
        blockId: block1,
        expectedRevision: 3,
        expectedHash: "sha256:base",
        nextOrderKey: "b",
      },
      {
        type: "delete_block",
        blockId: block1,
        expectedRevision: 4,
        expectedHash: "sha256:base",
      },
    ]), hasher),
    hasCode("BLOCK_TOUCHED_TWICE"),
  );
});

test("requires stable Tiptap block attributes", () => {
  assert.throws(
    () => fromTiptapSnapshot({
      documentId,
      revision: 1,
      blocks: [{ type: "paragraph", attrs: {}, json: {}, textContent: "missing id" }],
    }),
    hasCode("STABLE_BLOCK_ID_MISSING"),
  );
});

test("maps a valid Tiptap snapshot without depending on the editor runtime", () => {
  const mapped = fromTiptapSnapshot({
    documentId,
    revision: 9,
    blocks: [{
      type: "heading",
      attrs: {
        optimizerBlockId: block1,
        optimizerRevision: 4,
        optimizerContentHash: "sha256:heading",
        optimizerOrderKey: "01",
      },
      json: { type: "heading", attrs: { level: 2 } },
      textContent: "章节标题",
    }],
  });

  assert.equal(mapped.blocks[0]?.kind, "heading");
  assert.equal(mapped.blocks[0]?.id, block1);
  assert.equal(mapped.blocks[0]?.revision, 4);
});
