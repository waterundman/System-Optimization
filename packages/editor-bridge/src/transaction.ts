import { stableStringify, type BlockId } from "../../protocol/src/index.ts";
import { EditorBridgeError } from "./errors.ts";
import type {
  CompiledEditorTransaction,
  EditorBlockChange,
  EditorBlockSnapshot,
  EditorDocumentSnapshot,
  EditorHasher,
  EditorStep,
  EditorTransaction,
} from "./types.ts";

export async function compileEditorTransaction(
  document: EditorDocumentSnapshot,
  transaction: EditorTransaction,
  hasher: EditorHasher,
): Promise<CompiledEditorTransaction> {
  if (transaction.steps.length === 0) {
    throw new EditorBridgeError("EMPTY_TRANSACTION", "An editor transaction must contain at least one step");
  }
  if (transaction.documentId !== document.documentId || transaction.expectedDocumentRevision !== document.revision) {
    throw new EditorBridgeError(
      "DOCUMENT_REVISION_CONFLICT",
      "Editor transaction was created from a stale document revision",
      {
        expectedDocumentId: transaction.documentId,
        actualDocumentId: document.documentId,
        expectedRevision: transaction.expectedDocumentRevision,
        actualRevision: document.revision,
      },
    );
  }

  const touched = new Set<BlockId>();
  const blocks = new Map(document.blocks.map((block) => [block.id, block]));
  const changes: EditorBlockChange[] = [];

  for (const step of transaction.steps) {
    if (step.type === "insert_block" && step.block.documentId !== document.documentId) {
      throw new EditorBridgeError(
        "DOCUMENT_ID_CONFLICT",
        `Inserted block ${step.block.id} belongs to a different document`,
        { expectedDocumentId: document.documentId, actualDocumentId: step.block.documentId },
      );
    }
    const blockId = step.type === "insert_block" ? step.block.id : step.blockId;
    if (touched.has(blockId)) {
      throw new EditorBridgeError(
        "BLOCK_TOUCHED_TWICE",
        `A normalized transaction may touch block ${blockId} only once`,
      );
    }
    touched.add(blockId);
    changes.push(await applyStep(blocks, step, hasher));
  }

  const nextBlocks = [...blocks.values()].sort(compareBlocks);
  assertUniqueOrderKeys(nextBlocks);
  return {
    id: transaction.id,
    documentId: document.documentId,
    expectedDocumentRevision: document.revision,
    newDocumentRevision: document.revision + 1,
    changes,
    nextDocument: {
      documentId: document.documentId,
      revision: document.revision + 1,
      blocks: nextBlocks,
    },
  };
}

async function applyStep(
  blocks: Map<BlockId, EditorBlockSnapshot>,
  step: EditorStep,
  hasher: EditorHasher,
): Promise<EditorBlockChange> {
  if (step.type === "insert_block") {
    if (blocks.has(step.block.id)) {
      throw new EditorBridgeError("BLOCK_ID_DUPLICATE", `Block already exists: ${step.block.id}`);
    }
    const after: EditorBlockSnapshot = {
      ...step.block,
      contentHash: await hashBlock(step.block, hasher),
      revision: 0,
    };
    blocks.set(after.id, after);
    return { operation: "create", blockId: after.id, after };
  }

  const before = blocks.get(step.blockId);
  if (!before) {
    throw new EditorBridgeError("BLOCK_NOT_FOUND", `Block not found: ${step.blockId}`);
  }
  assertWritableCurrent(before, step.expectedRevision, step.expectedHash);

  if (step.type === "delete_block") {
    blocks.delete(before.id);
    return { operation: "delete", blockId: before.id, before };
  }

  if (step.type === "move_block") {
    if (!step.nextOrderKey.trim()) {
      throw new EditorBridgeError("ORDER_KEY_DUPLICATE", "A moved block requires a non-empty order key");
    }
    const after: EditorBlockSnapshot = {
      ...before,
      orderKey: step.nextOrderKey,
      revision: before.revision + 1,
    };
    blocks.set(after.id, after);
    return { operation: "move", blockId: before.id, before, after };
  }

  const hashInput = {
    kind: step.nextKind ?? before.kind,
    content: step.nextContent,
    plainText: step.nextPlainText,
    locked: before.locked,
  };
  const after: EditorBlockSnapshot = {
    ...before,
    kind: step.nextKind ?? before.kind,
    content: step.nextContent,
    plainText: step.nextPlainText,
    contentHash: await hashBlock(hashInput, hasher),
    revision: before.revision + 1,
  };
  blocks.set(after.id, after);
  return { operation: "update", blockId: before.id, before, after };
}

function assertWritableCurrent(
  block: EditorBlockSnapshot,
  expectedRevision: number,
  expectedHash: string,
): void {
  if (block.locked) {
    throw new EditorBridgeError("BLOCK_LOCKED", `Block is locked: ${block.id}`);
  }
  if (block.revision !== expectedRevision) {
    throw new EditorBridgeError("BLOCK_REVISION_CONFLICT", `Block revision changed: ${block.id}`, {
      expectedRevision,
      actualRevision: block.revision,
    });
  }
  if (block.contentHash !== expectedHash) {
    throw new EditorBridgeError("BLOCK_HASH_CONFLICT", `Block content hash changed: ${block.id}`, {
      expectedHash,
      actualHash: block.contentHash,
    });
  }
}

async function hashBlock(
  block: Pick<EditorBlockSnapshot, "kind" | "content" | "plainText" | "locked">,
  hasher: EditorHasher,
): Promise<string> {
  return hasher.sha256(
    stableStringify({
      kind: block.kind,
      content: block.content,
      plainText: block.plainText,
      locked: block.locked,
    }),
  );
}

function compareBlocks(a: EditorBlockSnapshot, b: EditorBlockSnapshot): number {
  return a.orderKey.localeCompare(b.orderKey) || a.id.localeCompare(b.id);
}

function assertUniqueOrderKeys(blocks: readonly EditorBlockSnapshot[]): void {
  for (let index = 1; index < blocks.length; index += 1) {
    if (blocks[index - 1]?.orderKey === blocks[index]?.orderKey) {
      throw new EditorBridgeError(
        "ORDER_KEY_DUPLICATE",
        `Duplicate order key: ${blocks[index]?.orderKey}`,
      );
    }
  }
}
