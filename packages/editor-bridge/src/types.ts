import type {
  BlockId,
  BlockKind,
  DocumentId,
  OperationTarget,
  TextAnchor,
} from "@optimizer/protocol";

export interface EditorBlockSnapshot {
  readonly id: BlockId;
  readonly documentId: DocumentId;
  readonly kind: BlockKind;
  readonly orderKey: string;
  readonly content: Readonly<Record<string, unknown>>;
  readonly plainText: string;
  readonly contentHash: string;
  readonly revision: number;
  readonly locked: boolean;
}

export interface EditorDocumentSnapshot {
  readonly documentId: DocumentId;
  readonly revision: number;
  readonly blocks: readonly EditorBlockSnapshot[];
}

export interface EditorSelectionSnapshot {
  readonly anchor: TextAnchor;
  readonly head: TextAnchor;
  readonly offsetEncoding: "utf16";
}

export type EditorStep =
  | {
      readonly type: "replace_block";
      readonly blockId: BlockId;
      readonly expectedRevision: number;
      readonly expectedHash: string;
      readonly nextKind?: BlockKind;
      readonly nextContent: Readonly<Record<string, unknown>>;
      readonly nextPlainText: string;
    }
  | {
      readonly type: "insert_block";
      readonly block: Omit<EditorBlockSnapshot, "contentHash" | "revision">;
    }
  | {
      readonly type: "delete_block";
      readonly blockId: BlockId;
      readonly expectedRevision: number;
      readonly expectedHash: string;
    }
  | {
      readonly type: "move_block";
      readonly blockId: BlockId;
      readonly expectedRevision: number;
      readonly expectedHash: string;
      readonly nextOrderKey: string;
    };

export interface EditorTransaction {
  readonly id: string;
  readonly documentId: DocumentId;
  readonly expectedDocumentRevision: number;
  readonly steps: readonly EditorStep[];
}

export interface EditorBlockChange {
  readonly operation: "create" | "update" | "delete" | "move";
  readonly blockId: BlockId;
  readonly before?: EditorBlockSnapshot;
  readonly after?: EditorBlockSnapshot;
}

export interface CompiledEditorTransaction {
  readonly id: string;
  readonly documentId: DocumentId;
  readonly expectedDocumentRevision: number;
  readonly newDocumentRevision: number;
  readonly changes: readonly EditorBlockChange[];
  readonly nextDocument: EditorDocumentSnapshot;
}

export interface EditorHasher {
  readonly id: string;
  sha256(value: string): Promise<string>;
}

export interface OperationTargetResult {
  readonly target: OperationTarget;
  readonly selectedText: string;
}

