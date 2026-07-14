import assert from "node:assert/strict";
import test from "node:test";
import { DomainError, OperationLifecycle, canTransition } from "../src/index.ts";

const at = "2026-07-14T00:00:00.000Z";

test("runs the successful operation lifecycle", () => {
  const lifecycle = new OperationLifecycle();
  for (const state of ["compiling", "preflight", "queued", "streaming", "validating", "review", "accepted"] as const) {
    lifecycle.transition(state, at);
  }
  assert.equal(lifecycle.state, "accepted");
  assert.equal(lifecycle.history.length, 7);
});

test("does not allow a terminal state to transition", () => {
  const lifecycle = new OperationLifecycle("accepted");
  assert.throws(
    () => lifecycle.transition("review", at),
    (error: unknown) => error instanceof DomainError && error.code === "OPERATION_INVALID_TRANSITION",
  );
});

test("allows a conflicted proposal to return to review after rebase", () => {
  assert.equal(canTransition("conflicted", "review"), true);
  assert.equal(canTransition("conflicted", "accepted"), false);
});

