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

test("T01: validating can transition to cancelled", () => {
  const lifecycle = new OperationLifecycle("validating");
  lifecycle.transition("cancelled", at);
  assert.equal(lifecycle.state, "cancelled");
});

test("T02: review can transition to cancelled with reason", () => {
  const lifecycle = new OperationLifecycle("review");
  lifecycle.transition("cancelled", at, "user_cancelled");
  assert.equal(lifecycle.state, "cancelled");
});

test("T03: cancelled is terminal and cannot transition out", () => {
  const lifecycle = new OperationLifecycle("cancelled");
  assert.throws(
    () => lifecycle.transition("accepted", at),
    (error: unknown) => error instanceof DomainError && error.code === "OPERATION_INVALID_TRANSITION",
  );
});

test("T04: regression - existing transitions still work", () => {
  const lifecycle = new OperationLifecycle();
  lifecycle.transition("compiling", at);
  lifecycle.transition("preflight", at);
  lifecycle.transition("cancelled", at);
  assert.equal(lifecycle.state, "cancelled");

  const streaming = new OperationLifecycle("streaming");
  streaming.transition("validating", at);
  assert.equal(streaming.state, "validating");
});

test("T05: canTransition returns true for new cancelled migrations", () => {
  assert.equal(canTransition("validating", "cancelled"), true);
  assert.equal(canTransition("review", "cancelled"), true);
});

