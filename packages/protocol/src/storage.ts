import type { BlockId, CommitId, DocumentId, ProjectId } from "./types.ts";

export type DocumentKind = "folder" | "document" | "chapter" | "scene" | "note";
export type BlockKind = "paragraph" | "heading" | "quote" | "dialogue" | "list" | "table" | "scene_break" | "locked";

export interface DocumentSnapshot {
  readonly id: DocumentId;
  readonly projectId: ProjectId;
  readonly parentId?: DocumentId;
  readonly kind: DocumentKind;
  readonly title: string;
  readonly orderKey: string;
  readonly revision: number;
}

export interface BlockSnapshot {
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

export interface CommitDescriptor {
  readonly id: CommitId;
  readonly projectId: ProjectId;
  readonly parentIds: readonly CommitId[];
  readonly rootHash: string;
  readonly reason: "initial" | "autosave" | "pre_ai" | "ai_accept" | "manual" | "restore" | "import" | "migration";
  readonly actorType: "system" | "human" | "model" | "plugin";
  readonly actorId?: string;
  readonly createdAt: string;
}

export interface MaterializedSnapshotDescriptor {
  readonly id: string;
  readonly projectId: ProjectId;
  readonly commitId: CommitId;
  readonly rootHash: string;
  readonly codec: string;
  readonly codecVersion: number;
  readonly checksum: string;
  readonly createdAt: string;
}

