export type EditorBridgeErrorCode =
  | "EMPTY_TRANSACTION"
  | "DOCUMENT_REVISION_CONFLICT"
  | "DOCUMENT_ID_CONFLICT"
  | "BLOCK_REVISION_CONFLICT"
  | "BLOCK_HASH_CONFLICT"
  | "BLOCK_NOT_FOUND"
  | "BLOCK_LOCKED"
  | "BLOCK_ID_DUPLICATE"
  | "ORDER_KEY_DUPLICATE"
  | "BLOCK_TOUCHED_TWICE"
  | "SELECTION_CROSSES_BLOCKS"
  | "SELECTION_OFFSET_INVALID"
  | "STABLE_BLOCK_ID_MISSING"
  | "TIPTAP_ATTR_INVALID";

export class EditorBridgeError extends Error {
  readonly code: EditorBridgeErrorCode;
  readonly details?: Readonly<Record<string, unknown>>;

  constructor(code: EditorBridgeErrorCode, message: string, details?: Readonly<Record<string, unknown>>) {
    super(message);
    this.name = "EditorBridgeError";
    this.code = code;
    this.details = details;
  }
}
