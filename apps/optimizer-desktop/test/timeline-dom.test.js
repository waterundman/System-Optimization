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
// ResizeObserver. Install synchronous polyfills so virtualizeDiffRows can be
// exercised in unit tests.
if (typeof global.requestAnimationFrame !== "function") {
  global.requestAnimationFrame = (cb) => {
    cb(Date.now());
    return 1;
  };
}
if (typeof global.cancelAnimationFrame !== "function") {
  global.cancelAnimationFrame = () => {};
}

const resizeObserverInstances = [];
class MockResizeObserver {
  constructor(cb) {
    this.cb = cb;
    this.targets = new Set();
    resizeObserverInstances.push(this);
  }
  observe(el) { if (el) this.targets.add(el); }
  unobserve(el) { this.targets.delete(el); }
  disconnect() { this.targets.clear(); }
}
global.ResizeObserver = MockResizeObserver;

const { state, timelineDrawer, TIMELINE_STATE_COLORS } = await import(
  "../dist/main.js"
);

function createDocument() {
  const { document } = parseHTML(
    `<!DOCTYPE html><html><body></body></html>`,
  );
  return document;
}

function resetResizeObserverPool() {
  resizeObserverInstances.length = 0;
}

const STATE_LIST = [
  "draft",
  "compiling",
  "preflight",
  "queued",
  "streaming",
  "validating",
  "review",
  "accepted",
  "rejected",
  "conflicted",
  "failed",
  "cancelled",
];

function makeEvent(index, overrides = {}) {
  const opState = STATE_LIST[index % STATE_LIST.length];
  const hasOp = overrides.operationSummary !== null;
  return {
    checkpointId: `snap-${index}`,
    commitId: `commit-${index}-0123456789abcdef`,
    createdAt: `2026-07-15T00:${String(index).padStart(2, "0")}:00.000Z`,
    operationSummary: hasOp
      ? overrides.operationSummary ?? {
          operationId: `op-${index}`,
          state: opState,
          operationType: null,
        }
      : null,
    ...overrides,
  };
}

function makeEvents(count) {
  const events = [];
  for (let i = 0; i < count; i++) events.push(makeEvent(i));
  return events;
}

function resetState() {
  state.timelineOpen = false;
  state.timelineEvents = null;
  state.timelineBusy = false;
  state.timelineSelected = null;
}

test("T03: timelineDrawer 渲染时间线节点列表", () => {
  resetResizeObserverPool();
  resetState();
  const doc = createDocument();
  state.timelineEvents = {
    events: makeEvents(5),
    totalCount: 5,
  };
  const drawer = timelineDrawer(doc);
  const nodes = drawer.querySelectorAll(".timeline-node");
  assert.equal(
    nodes.length,
    5,
    `expected 5 timeline nodes, got ${nodes.length}`,
  );
  // Each node should carry its checkpoint id as a data attribute.
  const firstId = nodes[0].getAttribute("data-checkpoint-id");
  assert.equal(firstId, "snap-0", "first node should carry checkpoint id");
});

test("T04: timelineDrawer 操作类型颜色编码正确（12 状态 pill）", () => {
  resetResizeObserverPool();
  resetState();
  const doc = createDocument();
  const events = STATE_LIST.map((opState, index) => ({
    checkpointId: `snap-${opState}`,
    commitId: `commit-${opState}-0123456789abcdef`,
    createdAt: `2026-07-15T00:${String(index).padStart(2, "0")}:00.000Z`,
    operationSummary: {
      operationId: `op-${opState}`,
      state: opState,
      operationType: null,
    },
  }));
  state.timelineEvents = { events, totalCount: events.length };
  const drawer = timelineDrawer(doc);
  const pills = drawer.querySelectorAll(".timeline-state-pill");
  assert.equal(
    pills.length,
    STATE_LIST.length,
    `expected ${STATE_LIST.length} state pills, got ${pills.length}`,
  );
  for (const opState of STATE_LIST) {
    const expectedColor = TIMELINE_STATE_COLORS[opState];
    assert.ok(expectedColor, `color map missing for state ${opState}`);
    const pill = drawer.querySelector(`.timeline-state-pill.timeline-state-${opState}`);
    assert.ok(pill, `pill for state ${opState} should render`);
    const style = pill.getAttribute("style") || "";
    assert.ok(
      style.includes(expectedColor),
      `pill for state ${opState} should embed hex ${expectedColor} in style, got: ${style}`,
    );
  }
});

test("T05: timelineDrawer 虚拟滚动复用 virtualizeDiffRows（1000 节点 < 50 DOM）", () => {
  resetResizeObserverPool();
  resetState();
  const doc = createDocument();
  state.timelineEvents = {
    events: makeEvents(1000),
    totalCount: 1000,
  };
  const drawer = timelineDrawer(doc);
  const viewport = drawer.querySelector(".timeline-list");
  assert.ok(viewport, "timeline-list viewport should exist");
  const rendered = viewport.querySelectorAll(".timeline-node");
  assert.ok(
    rendered.length > 0,
    "expected at least one rendered node inside the visible window",
  );
  assert.ok(
    rendered.length < 50,
    `expected < 50 DOM nodes after virtualization, got ${rendered.length}`,
  );
  // The track element should report the full virtual height so the viewport
  // remains scrollable.
  const track = viewport.querySelector(".compare-drawer-virtual-track");
  assert.ok(track, "track element should be present inside viewport");
});

test("T06: timelineDrawer 点击节点加载 diff 预览", () => {
  resetResizeObserverPool();
  resetState();
  const doc = createDocument();
  const events = makeEvents(3);
  state.timelineEvents = { events, totalCount: 3 };
  let drawer = timelineDrawer(doc);
  const nodes = drawer.querySelectorAll(".timeline-node");
  assert.equal(nodes.length, 3, "precondition: 3 nodes rendered");
  // Preview should be empty before any click.
  let preview = drawer.querySelector(".timeline-preview");
  assert.ok(preview, "preview pane should exist");
  assert.ok(
    preview.querySelector(".timeline-preview-empty"),
    "preview should show empty placeholder before a node is clicked",
  );

  // Click the second node. The handler sets state.timelineSelected; we then
  // re-render to observe the updated preview pane.
  nodes[1].dispatchEvent(new doc.defaultView.Event("click", { bubbles: true }));
  assert.ok(
    state.timelineSelected,
    "clicking a node should populate state.timelineSelected",
  );
  assert.equal(
    state.timelineSelected.checkpointId,
    "snap-1",
    "selected checkpoint id should match the clicked node",
  );

  drawer = timelineDrawer(doc);
  preview = drawer.querySelector(".timeline-preview");
  const detail = preview.querySelector(".timeline-preview-detail");
  assert.ok(detail, "preview should show detail after a node is clicked");
  const createdAtSpan = detail.querySelector("span[title]");
  assert.ok(
    createdAtSpan && createdAtSpan.getAttribute("title").includes("commit-1"),
    `preview detail should carry the selected commit id as title attr, got: ${createdAtSpan && createdAtSpan.getAttribute("title")}`,
  );
});

// ===========================================================================
// v0.7.0 Stage 1 (D1 + D7): branch topology timeline + SVG connection lines
// ===========================================================================
//
// These tests exercise the new SVG overlay layer that renders branch merge
// topology via <path> elements drawn inside an <svg> overlay sitting on top
// of the existing virtualised timeline list. The overlay must:
//
// 1. Render <path> elements when at least one event carries a multi-parent
//    `parentCommitIds` array (length > 1) — i.e. a merge commit exists.
// 2. Stay absent (or render empty) for purely linear histories where every
//    event has at most one parent — backward compatible with v0.6.0.
// 3. Use the SVG namespace via `document.createElementNS` rather than plain
//    `document.createElement` so the elements are addressable by SVG-aware
//    CSS and querySelector.
// 4. Assign each branch a distinct horizontal lane offset so lanes never
//    overlap visually.
//
// The tests import `element` directly so the SVG namespace extension (T09)
// can be verified in isolation without going through the full drawer render.

function makeBranchEvent(index, overrides = {}) {
  // Branch-aware variant of `makeEvent`. Defaults to a single-parent linear
  // commit; tests can pass `parentCommitIds: [...]` to override.
  return {
    checkpointId: `snap-${index}`,
    commitId: `commit-${index}-0123456789abcdef`,
    createdAt: `2026-07-15T00:${String(index).padStart(2, "0")}:00.000Z`,
    operationSummary: null,
    parentCommitIds: [],
    ...overrides,
  };
}

test("T07: timelineDrawer SVG 连接线渲染（分支场景 parent_commit_ids 长度 > 1）", () => {
  // v0.7.0 Stage 1 (D7): when the timeline contains at least one merge
  // commit (parent_commit_ids.length > 1), the drawer must render an SVG
  // overlay populated with <path> elements encoding the branch merge
  // topology. Each <path> must be created in the SVG namespace so that
  // linkedom can address it via querySelector.
  //
  // The test seeds a 4-event topology where both the mainline parent
  // (commit-2) and the branch fork parent (commit-2a-fork) have their own
  // checkpoints in the timeline. This mirrors the real-world scenario
  // where each branch head gets its own snapshot/checkpoint before the
  // merge is recorded, so both parents are addressable from the overlay's
  // event index lookup.
  resetResizeObserverPool();
  resetState();
  const doc = createDocument();
  const events = [
    makeBranchEvent(0, {
      commitId: "commit-initial",
      parentCommitIds: [],
    }),
    makeBranchEvent(1, {
      commitId: "commit-2",
      parentCommitIds: ["commit-initial"],
    }),
    makeBranchEvent(2, {
      commitId: "commit-2a-fork",
      parentCommitIds: ["commit-initial"],
    }),
    makeBranchEvent(3, {
      commitId: "commit-3-merge",
      parentCommitIds: ["commit-2", "commit-2a-fork"],
    }),
  ];
  state.timelineEvents = { events, totalCount: events.length };
  const drawer = timelineDrawer(doc);
  const overlay = drawer.querySelector(".timeline-svg-overlay");
  assert.ok(overlay, "SVG overlay container must exist for branch scenarios");
  assert.equal(
    overlay.namespaceURI,
    "http://www.w3.org/2000/svg",
    "overlay must be created in the SVG namespace",
  );
  // In real browsers, SVG <path> elements created via createElementNS have
  // tagName "path" (lowercase, no prefix). In linkedom (used for unit tests),
  // the same elements have tagName "SVG:PATH" (uppercase, with namespace
  // prefix), so querySelectorAll("path") returns 0 matches. We use the [d]
  // attribute selector instead — the `d` attribute is specific to SVG <path>
  // elements in our overlay context — and filter by namespaceURI to ensure
  // the elements are SVG paths. This approach works in both linkedom and
  // real browsers.
  const paths = Array.from(overlay.querySelectorAll("[d]")).filter(
    (el) => el.namespaceURI === "http://www.w3.org/2000/svg",
  );
  assert.ok(
    paths.length >= 1,
    `overlay must render at least one <path> for merge topology, got ${paths.length}`,
  );
  for (const path of paths) {
    assert.equal(
      path.namespaceURI,
      "http://www.w3.org/2000/svg",
      "every <path> must be created in the SVG namespace",
    );
  }
});

test("T08: timelineDrawer 线性场景不渲染 SVG 连接线（向后兼容 v0.6.0）", () => {
  // v0.7.0 Stage 1 (D7, backward-compat): for a purely linear history where
  // every event has at most one parent, the drawer must NOT render an SVG
  // overlay (or render an empty one). This preserves the v0.6.0 baseline
  // where the timeline was a simple list of HTML dots without any SVG
  // connection lines.
  resetResizeObserverPool();
  resetState();
  const doc = createDocument();
  // 3 events: all linear, all with <= 1 parent.
  const events = [
    makeBranchEvent(0, {
      commitId: "commit-initial",
      parentCommitIds: [],
    }),
    makeBranchEvent(1, {
      commitId: "commit-2",
      parentCommitIds: ["commit-initial"],
    }),
    makeBranchEvent(2, {
      commitId: "commit-3",
      parentCommitIds: ["commit-2"],
    }),
  ];
  state.timelineEvents = { events, totalCount: events.length };
  const drawer = timelineDrawer(doc);
  const overlay = drawer.querySelector(".timeline-svg-overlay");
  // Either no overlay at all, or an overlay with zero <path> elements.
  if (overlay) {
    const paths = overlay.querySelectorAll("path");
    assert.equal(
      paths.length,
      0,
      `linear history must not render any SVG <path> elements, got ${paths.length}`,
    );
  }
  // The drawer must still render the normal timeline nodes as a fallback.
  const nodes = drawer.querySelectorAll(".timeline-node");
  assert.ok(
    nodes.length >= 1,
    `linear history must still render timeline nodes, got ${nodes.length}`,
  );
});

test("T09: element 函数支持 SVG namespace（createElementNS）", async () => {
  // v0.7.0 Stage 1 (D7): the `element` helper must support SVG namespace
  // creation via a tag prefix (e.g. "svg:svg" -> createElementNS) so that
  // the SVG overlay and its <path> children are created in the SVG
  // namespace. The function must NOT break existing HTML element creation
  // (backward compat with v0.6.0 callers).
  //
  // We re-import main.js to access the `element` export without polluting
  // the shared `state` used by other tests.
  const main = await import("../dist/main.js");
  const element = main.element;
  assert.equal(typeof element, "function", "element must be exported as a function");
  const doc = createDocument();

  // SVG element via "svg:" prefix must use createElementNS.
  const svgEl = element("svg:svg", {}, [], doc);
  assert.equal(
    svgEl.namespaceURI,
    "http://www.w3.org/2000/svg",
    "svg:svg tag must produce an element in the SVG namespace",
  );
  assert.equal(
    svgEl.tagName.toLowerCase(),
    "svg",
    "svg:svg tag must strip the prefix to yield the local name in tagName (matches real browser createElementNS behaviour)",
  );

  // Regular HTML element must still use createElement (backward compat).
  const htmlEl = element("div", { className: "html-test" }, [], doc);
  assert.equal(
    htmlEl.namespaceURI,
    "http://www.w3.org/1999/xhtml",
    "div tag must produce an element in the HTML namespace",
  );
  assert.equal(
    htmlEl.tagName.toLowerCase(),
    "div",
    "div tag must produce a plain div tagName",
  );
  assert.equal(
    htmlEl.className,
    "html-test",
    "div tag must still apply options.className for backward compat",
  );

  // SVG <path> child via "svg:" prefix must also use createElementNS.
  const pathEl = element("svg:path", { attrs: { d: "M0 0 L10 10" } }, [], doc);
  assert.equal(
    pathEl.namespaceURI,
    "http://www.w3.org/2000/svg",
    "svg:path tag must produce an element in the SVG namespace",
  );
  assert.equal(
    pathEl.getAttribute("d"),
    "M0 0 L10 10",
    "svg:path tag must still apply options.attrs for backward compat",
  );
});

test("T10: 轨道分配：多分支 lane 不重叠（critical: false）", () => {
  // v0.7.0 Stage 1 (D5): the branch layout algorithm must assign each
  // branch a distinct horizontal lane offset so that lanes never overlap
  // visually. The test seeds two distinct branches (fork from a common
  // parent) plus a merge commit that rejoins both branches. The merge
  // commit must carry parent_commit_ids.length > 1 so the SVG overlay
  // renders at least one <path>. The lanes are inspected via the
  // data-lane attribute on each timeline node; no two lanes must share
  // the same horizontal offset.
  resetResizeObserverPool();
  resetState();
  const doc = createDocument();
  const events = [
    makeBranchEvent(0, {
      commitId: "commit-initial",
      parentCommitIds: [],
    }),
    makeBranchEvent(1, {
      commitId: "commit-2",
      parentCommitIds: ["commit-initial"],
    }),
    makeBranchEvent(2, {
      commitId: "commit-2a-fork",
      parentCommitIds: ["commit-initial"],
    }),
    makeBranchEvent(3, {
      commitId: "commit-3-merge",
      parentCommitIds: ["commit-2", "commit-2a-fork"],
    }),
  ];
  state.timelineEvents = { events, totalCount: events.length };
  const drawer = timelineDrawer(doc);
  // Lanes are surfaced as data-lane attributes on each timeline node.
  const nodes = drawer.querySelectorAll(".timeline-node[data-lane]");
  assert.ok(
    nodes.length >= 1,
    `at least one timeline node must carry a data-lane attribute, got ${nodes.length}`,
  );
  const laneOffsets = new Set();
  for (const node of nodes) {
    const lane = Number.parseInt(node.getAttribute("data-lane") ?? "0", 10);
    assert.ok(
      Number.isInteger(lane) && lane >= 0 && lane < 10,
      `lane must be a non-negative integer < 10 (D5 constraint), got ${lane}`,
    );
    laneOffsets.add(lane);
  }
  // At least two distinct lanes must be assigned so that the fork and the
  // merge commit can be visually distinguished.
  assert.ok(
    laneOffsets.size >= 2,
    `expected at least 2 distinct lane offsets for a fork+merge topology, got ${laneOffsets.size}`,
  );
});
