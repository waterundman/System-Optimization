import type { OperationState } from "../../protocol/src/index.ts";
import { DomainError } from "./errors.ts";

const transitions: Readonly<Record<OperationState, readonly OperationState[]>> = {
  draft: ["compiling", "cancelled"],
  compiling: ["preflight", "failed", "cancelled"],
  preflight: ["queued", "failed", "cancelled"],
  queued: ["streaming", "failed", "cancelled"],
  streaming: ["validating", "failed", "cancelled"],
  validating: ["review", "failed", "cancelled"],
  review: ["accepted", "rejected", "conflicted", "cancelled"],
  conflicted: ["review", "rejected"],
  accepted: [],
  rejected: [],
  failed: [],
  cancelled: [],
};

export interface OperationTransition {
  readonly from: OperationState;
  readonly to: OperationState;
  readonly occurredAt: string;
  readonly reason?: string;
}

export function canTransition(from: OperationState, to: OperationState): boolean {
  return transitions[from].includes(to);
}

export class OperationLifecycle {
  #state: OperationState;
  #history: OperationTransition[] = [];

  constructor(initial: OperationState = "draft") {
    this.#state = initial;
  }

  get state(): OperationState {
    return this.#state;
  }

  get history(): readonly OperationTransition[] {
    return [...this.#history];
  }

  transition(to: OperationState, occurredAt: string, reason?: string): OperationTransition {
    if (!canTransition(this.#state, to)) {
      throw new DomainError({
        code: "OPERATION_INVALID_TRANSITION",
        category: "conflict",
        message: `Cannot transition operation from ${this.#state} to ${to}`,
        details: { from: this.#state, to },
      });
    }
    const event: OperationTransition = { from: this.#state, to, occurredAt, reason };
    this.#state = to;
    this.#history.push(event);
    return event;
  }
}

