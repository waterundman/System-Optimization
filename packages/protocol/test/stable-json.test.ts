import assert from "node:assert/strict";
import test from "node:test";
import { createHash } from "node:crypto";
import { stableStringify } from "../src/stable-json.ts";
import { stableStringify as kernelStableStringify } from "../../kernel/src/stable-json.ts";
import { stableStringify as patchEngineStableStringify } from "../../patch-engine/src/stable-json.ts";

// T01: stableStringify 行为不变（迁移后既有测试通过）— regression
test("T01: sorts object keys recursively", () => {
  assert.equal(stableStringify({ b: 1, a: 2 }), stableStringify({ a: 2, b: 1 }));
  assert.equal(stableStringify({ b: 1, a: 2 }), '{"a":2,"b":1}');
});

test("T01: filters undefined values", () => {
  assert.equal(stableStringify({ a: 1, b: undefined }), stableStringify({ a: 1 }));
  assert.equal(stableStringify({ a: 1, b: undefined }), '{"a":1}');
});

test("T01: rejects non-finite numbers", () => {
  assert.throws(() => stableStringify({ a: Infinity }), TypeError);
  assert.throws(() => stableStringify({ a: -Infinity }), TypeError);
  assert.throws(() => stableStringify({ a: NaN }), TypeError);
  assert.throws(() => stableStringify(Infinity), TypeError);
});

test("T01: preserves array order (no sorting)", () => {
  assert.equal(stableStringify([3, 1, 2]), "[3,1,2]");
  assert.notEqual(stableStringify([3, 1, 2]), stableStringify([1, 2, 3]));
});

test("T01: recursively sorts nested objects", () => {
  assert.equal(
    stableStringify({ z: { b: 1, a: 2 }, a: 1 }),
    '{"a":1,"z":{"a":2,"b":1}}',
  );
});

test("T01: handles null, booleans, strings, numbers", () => {
  assert.equal(stableStringify(null), "null");
  assert.equal(stableStringify(true), "true");
  assert.equal(stableStringify("hi"), '"hi"');
  assert.equal(stableStringify(42), "42");
});

test("T01: handles arrays of objects with per-element key sorting", () => {
  assert.equal(
    stableStringify([{ b: 1, a: 2 }, { d: 3, c: 4 }]),
    '[{"a":2,"b":1},{"c":4,"d":3}]',
  );
});

// T02: kernel 与 patch-engine 引用同一 stableStringify — unit
test("T02: kernel and patch-engine re-export the identical stableStringify from protocol", () => {
  assert.equal(
    kernelStableStringify,
    stableStringify,
    "kernel stableStringify must be the same function object as protocol's",
  );
  assert.equal(
    patchEngineStableStringify,
    stableStringify,
    "patch-engine stableStringify must be the same function object as protocol's",
  );
  assert.equal(
    kernelStableStringify,
    patchEngineStableStringify,
    "kernel and patch-engine must share a single stableStringify",
  );
});

test("T02: behavior identical across protocol, kernel, and patch-engine import paths", () => {
  const sample = { b: [2, 1], a: { d: undefined, c: true } };
  assert.equal(kernelStableStringify(sample), stableStringify(sample));
  assert.equal(patchEngineStableStringify(sample), stableStringify(sample));
});

// T03: 内容哈希稳定 — unit
test("T03: identical ProjectSnapshot input yields identical stableStringify output and sha256", () => {
  const snapshotA = {
    schemaVersion: 1,
    projectId: "project-1",
    commitId: "commit-1",
    documents: [
      { id: "doc-1", title: "第一章", orderKey: "a", revision: 1 },
      { id: "doc-2", title: "第二章", orderKey: "b", revision: 2 },
    ],
    metadata: { author: "tester", tags: ["draft", "v1"], nested: { z: 1, a: 0 } },
  };
  // Same logical content, different key insertion order, same array order.
  const snapshotB = {
    metadata: { nested: { a: 0, z: 1 }, tags: ["draft", "v1"], author: "tester" },
    documents: [
      { revision: 1, orderKey: "a", title: "第一章", id: "doc-1" },
      { revision: 2, orderKey: "b", title: "第二章", id: "doc-2" },
    ],
    commitId: "commit-1",
    projectId: "project-1",
    schemaVersion: 1,
  };

  const strA = stableStringify(snapshotA);
  const strB = stableStringify(snapshotB);
  assert.equal(strA, strB, "reordered object keys must produce identical stable string");

  const hashA = createHash("sha256").update(strA).digest("hex");
  const hashB = createHash("sha256").update(strB).digest("hex");
  assert.equal(hashA, hashB, "identical string must yield identical sha256");
  assert.match(hashA, /^[a-f0-9]{64}$/);

  // Deterministic across repeated calls.
  assert.equal(stableStringify(snapshotA), strA);
  const hashAgain = createHash("sha256").update(stableStringify(snapshotA)).digest("hex");
  assert.equal(hashAgain, hashA);
});

test("T03: hash stability holds through kernel and patch-engine paths", () => {
  const payload = { op: "polish", target: { blockId: "b-1", offset: 4 }, constraints: [] };
  const viaProtocol = createHash("sha256").update(stableStringify(payload)).digest("hex");
  const viaKernel = createHash("sha256").update(kernelStableStringify(payload)).digest("hex");
  const viaPatchEngine = createHash("sha256").update(patchEngineStableStringify(payload)).digest("hex");
  assert.equal(viaKernel, viaProtocol);
  assert.equal(viaPatchEngine, viaProtocol);
});
