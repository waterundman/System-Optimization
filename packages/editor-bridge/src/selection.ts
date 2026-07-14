import type { OperationTarget, TextAnchor } from "../../protocol/src/index.ts";
import { EditorBridgeError } from "./errors.ts";
import type {
  EditorDocumentSnapshot,
  EditorSelectionSnapshot,
  OperationTargetResult,
} from "./types.ts";

export function selectionToOperationTarget(
  document: EditorDocumentSnapshot,
  selection: EditorSelectionSnapshot,
): OperationTargetResult {
  if (selection.anchor.blockId !== selection.head.blockId) {
    throw new EditorBridgeError(
      "SELECTION_CROSSES_BLOCKS",
      "MVP operations must target a selection within one stable block",
      { anchorBlockId: selection.anchor.blockId, headBlockId: selection.head.blockId },
    );
  }
  const block = document.blocks.find((candidate) => candidate.id === selection.anchor.blockId);
  if (!block) {
    throw new EditorBridgeError("BLOCK_NOT_FOUND", `Selection block not found: ${selection.anchor.blockId}`);
  }
  assertOffset(block.plainText, selection.anchor.offset);
  assertOffset(block.plainText, selection.head.offset);

  const fromOffset = Math.min(selection.anchor.offset, selection.head.offset);
  const toOffset = Math.max(selection.anchor.offset, selection.head.offset);
  const from: TextAnchor = { blockId: block.id, offset: fromOffset, affinity: "after" };
  const to: TextAnchor = { blockId: block.id, offset: toOffset, affinity: "before" };
  const target: OperationTarget = {
    documentId: document.documentId,
    blockId: block.id,
    baseRevision: block.revision,
    baseHash: block.contentHash,
    from,
    to,
  };
  return { target, selectedText: block.plainText.slice(fromOffset, toOffset) };
}

function assertOffset(text: string, offset: number): void {
  if (!Number.isInteger(offset) || offset < 0 || offset > text.length || splitsSurrogatePair(text, offset)) {
    throw new EditorBridgeError(
      "SELECTION_OFFSET_INVALID",
      `Selection offset ${offset} is not a valid UTF-16 boundary`,
      { offset, length: text.length },
    );
  }
}

function splitsSurrogatePair(text: string, offset: number): boolean {
  if (offset <= 0 || offset >= text.length) return false;
  const previous = text.charCodeAt(offset - 1);
  const next = text.charCodeAt(offset);
  return previous >= 0xd800 && previous <= 0xdbff && next >= 0xdc00 && next <= 0xdfff;
}

