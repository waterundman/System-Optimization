import type { BlockId, BlockKind, DocumentId } from "@optimizer/protocol";
import { EditorBridgeError } from "./errors.ts";
import type { EditorBlockSnapshot, EditorDocumentSnapshot } from "./types.ts";

export interface TiptapBlockNodeSnapshot {
  readonly type: string;
  readonly attrs: Readonly<Record<string, unknown>>;
  readonly json: Readonly<Record<string, unknown>>;
  readonly textContent: string;
}

export interface TiptapDocumentSnapshot {
  readonly documentId: DocumentId;
  readonly revision: number;
  readonly blocks: readonly TiptapBlockNodeSnapshot[];
}

const kindMap: Readonly<Record<string, BlockKind>> = {
  paragraph: "paragraph",
  heading: "heading",
  blockquote: "quote",
  dialogue: "dialogue",
  bulletList: "list",
  orderedList: "list",
  table: "table",
  sceneBreak: "scene_break",
  lockedBlock: "locked",
};

export function fromTiptapSnapshot(snapshot: TiptapDocumentSnapshot): EditorDocumentSnapshot {
  const blocks = snapshot.blocks.map((node, index) => mapNode(snapshot.documentId, node, index));
  const ids = new Set<string>();
  for (const block of blocks) {
    if (ids.has(block.id)) {
      throw new EditorBridgeError("BLOCK_ID_DUPLICATE", `Duplicate Tiptap block id: ${block.id}`);
    }
    ids.add(block.id);
  }
  return { documentId: snapshot.documentId, revision: snapshot.revision, blocks };
}

function mapNode(
  documentId: DocumentId,
  node: TiptapBlockNodeSnapshot,
  index: number,
): EditorBlockSnapshot {
  const id = node.attrs.optimizerBlockId;
  if (typeof id !== "string" || !id.trim()) {
    throw new EditorBridgeError(
      "STABLE_BLOCK_ID_MISSING",
      `Tiptap block ${index} does not contain attrs.optimizerBlockId`,
    );
  }
  const revision = node.attrs.optimizerRevision;
  const contentHash = node.attrs.optimizerContentHash;
  const orderKey = node.attrs.optimizerOrderKey;
  if (!Number.isInteger(revision) || Number(revision) < 0) {
    throw new EditorBridgeError("TIPTAP_ATTR_INVALID", `Invalid optimizerRevision for block ${id}`);
  }
  if (typeof contentHash !== "string" || !contentHash.trim()) {
    throw new EditorBridgeError("TIPTAP_ATTR_INVALID", `Invalid optimizerContentHash for block ${id}`);
  }
  if (typeof orderKey !== "string" || !orderKey.trim()) {
    throw new EditorBridgeError("TIPTAP_ATTR_INVALID", `Invalid optimizerOrderKey for block ${id}`);
  }
  const kind = kindMap[node.type];
  if (!kind) {
    throw new EditorBridgeError("TIPTAP_ATTR_INVALID", `Unsupported top-level Tiptap node: ${node.type}`);
  }
  return {
    id: id as BlockId,
    documentId,
    kind,
    orderKey,
    content: node.json,
    plainText: node.textContent,
    contentHash,
    revision: Number(revision),
    locked: node.attrs.optimizerLocked === true || kind === "locked",
  };
}

