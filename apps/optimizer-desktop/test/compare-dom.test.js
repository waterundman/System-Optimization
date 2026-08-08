import assert from "node:assert/strict";
import test from "node:test";
import { parseHTML } from "linkedom";

// Set up minimal browser globals before importing main.js so its top-level
// `document.querySelector("#app")` and `window.addEventListener` calls do not
// throw. Each individual test creates its own linkedom document and passes it
// directly to the function under test so the rendered tree is isolated from
// this bootstrap document.
const bootstrapDom = parseHTML(
  `<!DOCTYPE html><html><body><div id="app"></div></body></html>`,
);
global.document = bootstrapDom.document;
global.window = bootstrapDom.window;
if (!global.localStorage) {
  global.localStorage = {
    getItem: () => null,
    setItem: () => {},
    removeItem: () => {},
    clear: () => {},
  };
}
if (!global.crypto) {
  global.crypto = { subtle: {} };
}

// linkedom does not implement requestAnimationFrame / cancelAnimationFrame /
// ResizeObserver. Install synchronous polyfills so virtualizeDiffRows and
// fpsMonitor can be exercised in unit tests. Tests that need to observe the
// rAF scheduling behaviour can pass explicit overrides via the options bag.
if (typeof global.requestAnimationFrame !== "function") {
  global.requestAnimationFrame = (cb) => {
    cb(Date.now());
    return 1;
  };
}
if (typeof global.cancelAnimationFrame !== "function") {
  global.cancelAnimationFrame = () => {};
}

// Mock ResizeObserver: records every constructed instance so tests can fire
// callbacks on demand. Default observe() stores the target so entries can be
// synthesised with the correct `target` field.
const resizeObserverInstances = [];
class MockResizeObserver {
  constructor(cb) {
    this.cb = cb;
    this.targets = new Set();
    resizeObserverInstances.push(this);
  }
  observe(el) { if (el) this.targets.add(el); }
  unobserve(el) { if (el) this.targets.delete(el); }
  disconnect() { this.targets.clear(); }
}
global.ResizeObserver = MockResizeObserver;

const {
  compareBlockRow,
  fpsMonitor,
  virtualizeDiffRows,
} = await import("../dist/main.js");

function createDocument() {
  const { document } = parseHTML(
    `<!DOCTYPE html><html><body></body></html>`,
  );
  return document;
}

function makeBlock(index, overrides = {}) {
  return {
    blockIdA: `blk-a-${index}`,
    blockIdB: `blk-b-${index}`,
    kind: index % 4 === 0 ? "added" : index % 4 === 1 ? "removed" : index % 4 === 2 ? "modified" : "unchanged",
    textDiff: [{ equal: `block ${index} content` }],
    ...overrides,
  };
}

function makeBlocks(count) {
  const blocks = [];
  for (let i = 0; i < count; i++) blocks.push(makeBlock(i));
  return blocks;
}

function resetResizeObserverPool() {
  resizeObserverInstances.length = 0;
}

test("T01: virtualizeDiffRows 仅渲染可见区 + overscan（±5）", () => {
  resetResizeObserverPool();
  const doc = createDocument();
  const container = doc.createElement("div");
  container.className = "compare-drawer-virtual";
  doc.body.append(container);
  const blocks = makeBlocks(100);
  const controller = virtualizeDiffRows(blocks, container, doc, compareBlockRow, {
    overscan: 5,
    estimateHeight: 48,
    viewportHeight: 240, // 240 / 48 = 5 visible rows
  });
  controller.update(0, 240);

  // With scrollTop=0 and 5 visible rows + ±5 overscan, expected range is
  // [0, 10) → 10 rendered rows. Allow a small margin but enforce the
  // critical invariant: far fewer than the full 100 blocks.
  const rendered = container.querySelectorAll(".compare-block");
  assert.ok(
    rendered.length > 0 && rendered.length <= 20,
    `expected <= 20 rendered rows for visible + overscan, got ${rendered.length}`,
  );
  assert.ok(
    rendered.length < blocks.length,
    `expected virtualization to reduce DOM, got ${rendered.length} of ${blocks.length}`,
  );
  const range = controller._computeVisibleRange(0, 240);
  assert.equal(range.start, 0, "scrollTop=0 should clamp start to 0");
  assert.ok(
    range.end <= 11,
    `visible+overscan end should not exceed ~10, got ${range.end}`,
  );
  controller.destroy();
});

test("T02: 行高缓存正确更新（ResizeObserver 触发后 offset 重算）", () => {
  resetResizeObserverPool();
  const doc = createDocument();
  const container = doc.createElement("div");
  doc.body.append(container);
  const blocks = makeBlocks(20);
  const controller = virtualizeDiffRows(blocks, container, doc, compareBlockRow, {
    overscan: 5,
    estimateHeight: 48,
    viewportHeight: 480,
  });
  controller.update(0, 480);

  // Initial state: no measured heights, all rows use the 48px estimate.
  assert.equal(controller._rowHeightCache.size, 0, "height cache should start empty");
  assert.equal(controller._offsetFor(1), 48, "initial offset(1) should equal estimateHeight");

  // Synthesise a ResizeObserver entry for the first rendered row reporting
  // a measured height of 80px.
  const firstObserver = resizeObserverInstances[0];
  assert.ok(firstObserver, "at least one ResizeObserver should be attached");
  const firstTarget = Array.from(firstObserver.targets)[0];
  assert.ok(firstTarget, "observer should have a target");
  firstObserver.cb([{ target: firstTarget, contentRect: { height: 80 } }]);

  assert.equal(
    controller._rowHeightCache.get(0),
    80,
    "height cache for index 0 should update to 80 after RO callback",
  );
  // offsetCache must be invalidated so offsetFor(1) reflects the new height.
  assert.equal(
    controller._offsetFor(1),
    80,
    "offset(1) should be recomputed as 80 after cache invalidation",
  );
  assert.equal(
    controller._offsetFor(2),
    128,
    "offset(2) should be 80 + 48 (estimate for index 1)",
  );
  controller.destroy();
});

test("T03: 二分查找滚动位置正确（O(log n)）", () => {
  resetResizeObserverPool();
  const doc = createDocument();
  const container = doc.createElement("div");
  doc.body.append(container);
  const blocks = makeBlocks(1000);
  const controller = virtualizeDiffRows(blocks, container, doc, compareBlockRow, {
    overscan: 5,
    estimateHeight: 48,
    viewportHeight: 600,
  });

  // Pre-populate the height cache with a uniform 50px per row so we have a
  // predictable offset table (offset[i] = i * 50). This lets us verify the
  // binary search against a linear reference implementation.
  for (let i = 0; i < blocks.length; i++) controller._rowHeightCache.set(i, 50);
  controller._offsetCache.clear();

  const cases = [0, 50, 249, 2500, 2501, 12345, 49999, 50000];
  for (const scrollTop of cases) {
    // Linear reference: first index whose top + height > scrollTop.
    let expected = 0;
    while (expected < blocks.length && expected * 50 + 50 <= scrollTop) expected++;
    const actual = controller._findFirstVisible(scrollTop);
    assert.equal(
      actual,
      expected,
      `findFirstVisible(${scrollTop}) should be ${expected}, got ${actual}`,
    );
  }

  // Explicit: scrollTop=2500 → row 50 (top=2500, top+height=2550 > 2500).
  assert.equal(
    controller._findFirstVisible(2500),
    50,
    "scrollTop=2500 should resolve to firstVisible=50",
  );
  // scrollTop=50000 (one past the end) should clamp to blocks.length.
  assert.equal(
    controller._findFirstVisible(50000),
    blocks.length,
    "scrollTop beyond total height should clamp to blocks.length",
  );
  controller.destroy();
});

test("T04: 1000 行虚拟化后 DOM 节点 < 50", () => {
  resetResizeObserverPool();
  const doc = createDocument();
  const container = doc.createElement("div");
  doc.body.append(container);
  const blocks = makeBlocks(1000);
  const controller = virtualizeDiffRows(blocks, container, doc, compareBlockRow, {
    overscan: 5,
    estimateHeight: 48,
    viewportHeight: 600,
  });

  // Scroll into the middle of the list: total = 1000 * 48 = 48000.
  // scrollTop=24000 → firstVisible≈500, visible ~13 rows + 5+5 overscan.
  controller.update(24000, 600);

  const rendered = container.querySelectorAll(".compare-block");
  assert.ok(
    rendered.length < 50,
    `expected < 50 DOM nodes after virtualization, got ${rendered.length}`,
  );
  assert.ok(
    rendered.length > 0,
    "expected at least one rendered row inside the visible window",
  );

  // The track element should report the full virtual height so the
  // viewport remains scrollable.
  const track = container.querySelector(".compare-drawer-virtual-track");
  assert.ok(track, "track element should be present");
  assert.equal(
    track.style.height,
    "48000px",
    `track height should equal total virtual height, got ${track.style.height}`,
  );
  controller.destroy();
});

test("T05: fpsMonitor 在非 dev 模式不显示", () => {
  const doc = createDocument();
  // Ensure dev flag is not set.
  const previous = global.window.__OPTIMIZER_DEV__;
  delete global.window.__OPTIMIZER_DEV__;

  try {
    const indicator = fpsMonitor(doc);
    assert.equal(
      indicator,
      null,
      "fpsMonitor should return null when window.__OPTIMIZER_DEV__ is not set",
    );
    // Even if dev flag is explicitly false, it should still return null.
    global.window.__OPTIMIZER_DEV__ = false;
    assert.equal(
      fpsMonitor(doc),
      null,
      "fpsMonitor should return null when window.__OPTIMIZER_DEV__ is falsy",
    );
  } finally {
    if (previous === undefined) delete global.window.__OPTIMIZER_DEV__;
    else global.window.__OPTIMIZER_DEV__ = previous;
  }
});
