import { createHash } from "node:crypto";
import type {
  BlockId,
  CommitId,
  DocumentId,
  OperationIntent,
  OperationIntentId,
  ProjectId,
} from "@optimizer/protocol";
import type { Clock, ContentHasher, IdGenerator, TokenEstimator } from "@optimizer/kernel";

export class FixedClock implements Clock {
  private readonly value: string;
  constructor(value = "2026-07-14T00:00:00.000Z") {
    this.value = value;
  }
  now(): string {
    return this.value;
  }
}

export class SequentialIds implements IdGenerator {
  private value = 0;
  next(prefix: "ctx" | "event" | "proposal"): string {
    this.value += 1;
    return `${prefix}_test_${this.value}`;
  }
}

export class ApproxTokenEstimator implements TokenEstimator {
  readonly id = "test:unicode-quarter-v1";
  estimate(text: string): number {
    return Math.max(1, Math.ceil([...text].length / 4));
  }
}

export class Sha256Hasher implements ContentHasher {
  readonly id = "test:node-sha256";
  async sha256(value: string): Promise<string> {
    return `sha256:${createHash("sha256").update(value, "utf8").digest("hex")}`;
  }
}

export function createIntent(overrides: Partial<OperationIntent> = {}): OperationIntent {
  const blockId = "block-1" as BlockId;
  return {
    schemaVersion: 1,
    id: "intent-1" as OperationIntentId,
    projectId: "project-1" as ProjectId,
    baseCommitId: "commit-1" as CommitId,
    type: "continue_scene",
    strength: "medium",
    target: {
      documentId: "document-1" as DocumentId,
      blockId,
      baseRevision: 3,
      baseHash: "sha256:base",
      from: { blockId, offset: 0 },
      to: { blockId, offset: 12 },
    },
    constraints: [{ severity: "hard", rule: "不得提前揭示凶手" }],
    output: { kind: "patch_proposal", maxTokens: 200 },
    createdAt: "2026-07-14T00:00:00.000Z",
    ...overrides,
  };
}
