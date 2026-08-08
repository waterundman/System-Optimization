export const SAVE_DEBOUNCE_MS = 900;

export function normalizeHostError(error) {
  if (error && typeof error === "object") {
    const code = typeof error.code === "string" ? error.code : "HOST_ERROR";
    const message = typeof error.message === "string" ? error.message : "本地宿主命令执行失败";
    return { code, message };
  }
  if (typeof error === "string") return { code: "HOST_ERROR", message: error };
  return { code: "HOST_ERROR", message: "本地宿主命令执行失败" };
}

export function selectInitialDocument(workspace, preferredDocumentId) {
  if (!workspace?.documents?.length) return null;
  if (preferredDocumentId && workspace.documents.some((item) => item.id === preferredDocumentId)) {
    return preferredDocumentId;
  }
  return [...workspace.documents]
    .sort((left, right) => left.orderKey.localeCompare(right.orderKey) || left.id.localeCompare(right.id))[0]?.id ?? null;
}

export function blocksForDocument(workspace, documentId) {
  return (workspace?.blocks ?? [])
    .filter((block) => block.documentId === documentId)
    .sort((left, right) => left.orderKey.localeCompare(right.orderKey) || left.id.localeCompare(right.id));
}

export function isEditableBlock(block) {
  return !block.locked && ["paragraph", "heading", "quote", "dialogue"].includes(block.kind);
}

export function tiptapContentFromPlainText(block, plainText) {
  const type = block.kind === "quote" ? "blockquote" : block.kind;
  const content = plainText.length > 0 ? [{ type: "text", text: plainText }] : [];
  const next = { type, content };
  if (block.kind === "heading" && block.content?.attrs && typeof block.content.attrs === "object") {
    next.attrs = block.content.attrs;
  }
  return next;
}

export function buildSaveBlockRequest(block, plainText) {
  if (!block?.id || !Number.isInteger(block.revision) || typeof block.contentHash !== "string") {
    throw new TypeError("A persisted block baseline is required before saving");
  }
  return {
    schemaVersion: 1,
    blockId: block.id,
    expectedRevision: block.revision,
    expectedHash: block.contentHash,
    content: tiptapContentFromPlainText(block, plainText),
    plainText,
  };
}

export function applySaveResponse(workspace, response) {
  if (!workspace || !response?.block) throw new TypeError("A workspace and save response are required");
  return {
    ...workspace,
    headCommitId: response.headCommitId,
    revision: response.projectRevision,
    documents: workspace.documents.map((document) =>
      document.id === response.block.documentId
        ? { ...document, revision: document.revision + 1 }
        : document,
    ),
    blocks: workspace.blocks.map((block) =>
      block.id === response.block.id ? response.block : block,
    ),
  };
}

export function countVisibleCharacters(workspace) {
  // Note: whitespace removal via a global regex is O(n) per block; counting
  // the resulting string's UTF-16 length avoids materialising a spread
  // character array per block (which doubled allocation for large workspaces).
  return (workspace?.blocks ?? []).reduce(
    (total, block) => total + block.plainText.replace(/\s/gu, "").length,
    0,
  );
}
