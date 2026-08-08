import assert from "node:assert/strict";
import test from "node:test";
import { parseHTML } from "linkedom";

// v0.8.0 Stage 3 (a11y): DOM integration tests for the drawer / command
// palette / patch review keyboard paths. These tests exercise the
// main.js render functions end-to-end via linkedom and verify:
//   T04 — drawer containers carry role=dialog + aria-modal=true + aria-labelledby
//   T05 — Escape closes drawer and restores focus
//   T06 — command palette ArrowUp/ArrowDown + Enter execute selected
//   T07 — command palette container has role=listbox + aria-activedescendant
//   T08 — patch review Tab cycles hunks + Enter accepts the current hunk
//   T09 — patch review state changes are announced through announceLive

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
if (typeof global.requestAnimationFrame !== "function") {
  global.requestAnimationFrame = (cb) => { cb(Date.now()); return 1; };
}
if (typeof global.cancelAnimationFrame !== "function") {
  global.cancelAnimationFrame = () => {};
}
if (typeof global.ResizeObserver !== "function") {
  global.ResizeObserver = class {
    observe() {}
    unobserve() {}
    disconnect() {}
  };
}

const {
  state,
  timelineDrawer,
  insightsDrawer,
  compareDrawer,
  backupRestoreWizard,
  commandPaletteModal,
  aiReviewView,
  announceReviewState,
} = await import("../dist/main.js");

function createDocument() {
  const dom = parseHTML(`<!DOCTYPE html><html><body></body></html>`);
  return dom.document;
}

// linkedom does not update document.activeElement when an element calls
// focus(). To keep tests deterministic we install a stub so trapFocus and
// the keyboard handlers can read the current focus target.
function installActiveElementStub(document) {
  const slot = { active: null };
  Object.defineProperty(document, "activeElement", {
    configurable: true,
    get() { return slot.active; },
  });
  const patch = (el) => {
    if (!el) return el;
    el.focus = function focus() { slot.active = el; };
    el.blur = function blur() { if (slot.active === el) slot.active = null; };
    return el;
  };
  const patchAll = (root) => {
    if (!root || !root.querySelectorAll) return;
    patch(root);
    for (const el of root.querySelectorAll("*")) patch(el);
  };
  return { slot, patch, patchAll };
}

// linkedom does not expose a KeyboardEvent constructor on its defaultView.
// Build the event with the standard Event constructor and define key /
// shiftKey on it so the keydown handlers can read them. preventDefault is
// overridden so tests can observe whether the handler called it (linkedom's
// native Event does support preventDefault, but we override for parity with
// a11y.test.js and to guarantee defaultPrevented is observable).
function makeKeyEvent(doc, key, { shiftKey = false } = {}) {
  const event = new doc.defaultView.Event("keydown", {
    bubbles: true,
    cancelable: true,
  });
  Object.defineProperty(event, "key", { value: key, configurable: true });
  Object.defineProperty(event, "shiftKey", { value: shiftKey, configurable: true });
  event.preventDefault = function preventDefault() {
    Object.defineProperty(this, "defaultPrevented", {
      value: true,
      configurable: true,
    });
  };
  return event;
}

function resetTimelineState() {
  state.timelineOpen = false;
  state.timelineEvents = null;
  state.timelineBusy = false;
  state.timelineSelected = null;
}

function resetInsightsState() {
  state.insightsOpen = false;
  state.insightsData = null;
  state.insightsBusy = null;
  state.showRevisionMetrics = false;
}

function resetCompareState() {
  state.compareOpen = false;
  state.compareBusy = false;
  state.compareResult = null;
  state.versionHistory = null;
  state.compareSelection = {
    snapshotIdA: "",
    snapshotIdB: "",
    documentIdA: "",
    documentIdB: "",
  };
}

function resetBackupWizardState() {
  state.backupWizardOpen = false;
  state.backupWizardStep = 1;
  state.backupWizardMode = null;
  state.backupWizardBusy = false;
  state.backupWizardResult = null;
  state.backupWizardManifest = null;
  state.backupWizardWarnings = [];
  state.backupWizardForm = {
    includeEndpoints: true,
    includeRecent: true,
    outputPath: "",
    archivePath: "",
    targetDirectory: "",
    newProjectId: "",
    overwrite: false,
  };
}

function resetCommandPaletteState() {
  state.commandPaletteOpen = false;
  state.aiContextMenu = null;
  state.workspace = { documents: [], blocks: [] };
  state.aiRunning = null;
  state.aiReview = null;
}

function resetReviewState() {
  state.aiReview = null;
  state.workspace = { documents: [], blocks: [] };
  state.aiRunning = null;
  state.candidateBusy = null;
}

// T04: every drawer exposes role=dialog + aria-modal=true + aria-labelledby.
test("T04: every drawer container has role=dialog + aria-modal=true + aria-labelledby", () => {
  const doc = createDocument();

  // timelineDrawer
  resetTimelineState();
  state.timelineOpen = true;
  state.timelineEvents = {
    events: [
      {
        checkpointId: "snap-0",
        commitId: "commit-0-0123456789abcdef",
        createdAt: "2026-08-03T00:00:00.000Z",
        operationSummary: null,
      },
    ],
    totalCount: 1,
  };
  const timeline = timelineDrawer(doc);
  assert.ok(timeline, "timelineDrawer must return a drawer element");
  assert.equal(timeline.getAttribute("role"), "dialog", "timelineDrawer role=dialog");
  assert.equal(timeline.getAttribute("aria-modal"), "true", "timelineDrawer aria-modal=true");
  assert.ok(
    timeline.getAttribute("aria-labelledby"),
    "timelineDrawer must reference aria-labelledby id",
  );

  // insightsDrawer
  resetInsightsState();
  state.insightsOpen = true;
  state.insightsData = { summary: {}, recentRuns: [] };
  const insights = insightsDrawer(doc);
  assert.ok(insights, "insightsDrawer must return a drawer element");
  assert.equal(insights.getAttribute("role"), "dialog", "insightsDrawer role=dialog");
  assert.equal(insights.getAttribute("aria-modal"), "true", "insightsDrawer aria-modal=true");
  assert.ok(insights.getAttribute("aria-labelledby"), "insightsDrawer aria-labelledby");

  // compareDrawer
  resetCompareState();
  state.compareOpen = true;
  state.versionHistory = {
    checkpoints: [
      { id: "snap-a", commitId: "commit-a-0123456789abcdef", createdAt: "2026-08-03T00:00:00.000Z" },
      { id: "snap-b", commitId: "commit-b-0123456789abcdef", createdAt: "2026-08-03T01:00:00.000Z" },
    ],
  };
  state.workspace = {
    documents: [{ id: "doc-1", title: "Doc 1" }],
    blocks: [],
  };
  state.compareSelection = {
    snapshotIdA: "snap-a",
    snapshotIdB: "snap-b",
    documentIdA: "doc-1",
    documentIdB: "doc-1",
  };
  const compare = compareDrawer(doc);
  assert.ok(compare, "compareDrawer must return a drawer element");
  assert.equal(compare.getAttribute("role"), "dialog", "compareDrawer role=dialog");
  assert.equal(compare.getAttribute("aria-modal"), "true", "compareDrawer aria-modal=true");
  assert.ok(compare.getAttribute("aria-labelledby"), "compareDrawer aria-labelledby");

  // backupRestoreWizard
  resetBackupWizardState();
  state.backupWizardOpen = true;
  const wizard = backupRestoreWizard(doc);
  assert.ok(wizard, "backupRestoreWizard must return a drawer element");
  assert.equal(wizard.getAttribute("role"), "dialog", "backupRestoreWizard role=dialog");
  assert.equal(wizard.getAttribute("aria-modal"), "true", "backupRestoreWizard aria-modal=true");
  assert.ok(wizard.getAttribute("aria-labelledby"), "backupRestoreWizard aria-labelledby");
});

// T05: Escape closes drawer and restores focus to the opener.
test("T05: Escape closes drawer and restores focus to the opener", () => {
  const doc = createDocument();
  const { slot, patchAll } = installActiveElementStub(doc);
  resetTimelineState();
  state.timelineOpen = true;
  state.timelineEvents = {
    events: [
      {
        checkpointId: "snap-0",
        commitId: "commit-0-0123456789abcdef",
        createdAt: "2026-08-03T00:00:00.000Z",
        operationSummary: null,
      },
    ],
    totalCount: 1,
  };
  const drawer = timelineDrawer(doc);
  patchAll(drawer);
  // Simulate the toolbar toggle button being focused before the drawer opened.
  const opener = doc.createElement("button");
  opener.textContent = "Toggle";
  patchAll(opener);
  slot.active = opener;
  // The drawer's keydown handler (registered by main.js) must close on Esc.
  const escape = makeKeyEvent(doc, "Escape");
  drawer.dispatchEvent(escape);
  assert.ok(escape.defaultPrevented, "Escape on the drawer must be preventDefault'd");
  assert.equal(state.timelineOpen, false, "Escape must close the timeline drawer");
  // The opener must regain focus.
  assert.equal(slot.active, opener, "Escape must restore focus to the opener");
});

// T06: command palette supports ArrowUp/ArrowDown navigation + Enter executes.
test("T06: command palette ArrowUp/ArrowDown + Enter executes the selected command", () => {
  const doc = createDocument();
  const { slot, patchAll } = installActiveElementStub(doc);
  resetCommandPaletteState();
  state.commandPaletteOpen = true;
  // Provide enough workspace so commandPaletteModal renders.
  state.workspace = {
    documents: [{ id: "doc-1", title: "Doc 1" }],
    blocks: [{ id: "blk-1", kind: "text", plainText: "hello", revision: 1, contentHash: "h-1" }],
  };
  let executed = null;
  // Inject a stub for executeRegisteredCommand by overriding the query input's
  // behaviour via the rendered rows: each row's click handler calls
  // executeRegisteredCommand; we instead intercept clicks on the row button.
  const backdrop = commandPaletteModal(doc);
  assert.ok(backdrop, "commandPaletteModal must render a backdrop");
  patchAll(backdrop);
  const list = backdrop.querySelector(".command-palette-list");
  assert.ok(list, "command palette list must exist");
  const options = list.querySelectorAll('.command-palette-item');
  assert.ok(options.length >= 1, "command palette must render at least one option");
  // Replace each option's click() with a recorder so Enter triggers our stub
  // instead of trying to invoke the host.
  for (const opt of options) {
    opt.click = function click() { executed = opt; };
  }
  const query = backdrop.querySelector("#command-palette-query");
  assert.ok(query, "command palette query input must exist");
  slot.active = query;

  // Initial active index: 0 (the first option).
  assert.equal(
    list.getAttribute("aria-activedescendant"),
    options[0].id,
    "aria-activedescendant must reference the first option initially",
  );
  assert.equal(
    options[0].getAttribute("aria-selected"),
    "true",
    "first option must be aria-selected=true initially",
  );

  // ArrowDown moves selection to the second option.
  const down = makeKeyEvent(doc, "ArrowDown");
  query.dispatchEvent(down);
  assert.ok(down.defaultPrevented, "ArrowDown must be preventDefault'd");
  if (options.length > 1) {
    assert.equal(
      list.getAttribute("aria-activedescendant"),
      options[1].id,
      "ArrowDown must move aria-activedescendant to the second option",
    );
    assert.equal(
      options[0].getAttribute("aria-selected"),
      "false",
      "first option must be unselected after ArrowDown",
    );
    assert.equal(
      options[1].getAttribute("aria-selected"),
      "true",
      "second option must be selected after ArrowDown",
    );

    // ArrowUp wraps back to the first option.
    const up = makeKeyEvent(doc, "ArrowUp");
    query.dispatchEvent(up);
    assert.ok(up.defaultPrevented, "ArrowUp must be preventDefault'd");
    assert.equal(
      list.getAttribute("aria-activedescendant"),
      options[0].id,
      "ArrowUp must move aria-activedescendant back to the first option",
    );
  }

  // Enter executes the currently selected option (the first one).
  const enter = makeKeyEvent(doc, "Enter");
  query.dispatchEvent(enter);
  assert.ok(enter.defaultPrevented, "Enter must be preventDefault'd");
  assert.ok(executed, "Enter must trigger execution of the selected option");
});

// T07: command palette container has role=listbox + aria-activedescendant;
// options have role=option + aria-selected.
test("T07: command palette list has role=listbox + aria-activedescendant, options have role=option + aria-selected", () => {
  const doc = createDocument();
  resetCommandPaletteState();
  state.commandPaletteOpen = true;
  state.workspace = {
    documents: [{ id: "doc-1", title: "Doc 1" }],
    blocks: [],
  };
  const backdrop = commandPaletteModal(doc);
  const list = backdrop.querySelector(".command-palette-list");
  assert.ok(list, "command palette list must exist");
  assert.equal(list.getAttribute("role"), "listbox", "list role must be listbox");
  assert.ok(
    list.getAttribute("aria-activedescendant"),
    "listbox must carry aria-activedescendant pointing at the active option",
  );
  const options = list.querySelectorAll(".command-palette-item");
  for (const opt of options) {
    assert.equal(opt.getAttribute("role"), "option", "each item role must be option");
    assert.ok(
      opt.getAttribute("aria-selected") === "true" || opt.getAttribute("aria-selected") === "false",
      "each option must declare aria-selected true/false",
    );
    assert.ok(opt.id, "each option must have an id so aria-activedescendant can reference it");
  }
});

// T08: patch review supports Tab between hunks + Enter accepts the current hunk.
test("T08: patch review Tab cycles hunks + Enter accepts the current hunk", () => {
  const doc = createDocument();
  const { slot, patchAll } = installActiveElementStub(doc);
  resetReviewState();
  state.aiReview = {
    kind: "patch_proposal",
    busy: false,
    session: {
      revision: 1,
      status: "review",
      decisions: {},
    },
    result: {
      kind: "patch_proposal",
      proposal: {
        id: "prop-1",
        summary: "test",
        hunks: [
          { id: "hunk-1", granularity: "block", original: "old", replacement: "new" },
          { id: "hunk-2", granularity: "block", original: "old2", replacement: "new2" },
        ],
      },
    },
  };
  state.workspace = {
    documents: [{ id: "doc-1", title: "Doc 1" }],
    blocks: [],
  };
  const view = aiReviewView(doc);
  assert.ok(view, "aiReviewView must return a section element");
  patchAll(view);
  // The hunk articles must have role=group + aria-label.
  const hunks = view.querySelectorAll(".review-hunk");
  assert.ok(hunks.length >= 2, "must render at least 2 hunks");
  for (const hunk of hunks) {
    assert.equal(hunk.getAttribute("role"), "group", "hunk role=group");
    assert.ok(hunk.getAttribute("aria-label"), "hunk must have aria-label");
    // Each hunk must be focusable (tabindex=0) so Tab can land on it.
    assert.equal(hunk.getAttribute("tabindex"), "0", "hunk must have tabindex=0");
  }
  // Tab from the last hunk wraps to the first.
  slot.active = hunks[1];
  const tab = makeKeyEvent(doc, "Tab");
  hunks[1].dispatchEvent(tab);
  assert.ok(tab.defaultPrevented, "Tab on last hunk must be preventDefault'd");
  assert.equal(slot.active, hunks[0], "Tab from last hunk must wrap to first");
  // Shift+Tab from the first hunk wraps to the last.
  slot.active = hunks[0];
  const shiftTab = makeKeyEvent(doc, "Tab", { shiftKey: true });
  hunks[0].dispatchEvent(shiftTab);
  assert.ok(shiftTab.defaultPrevented, "Shift+Tab on first hunk must be preventDefault'd");
  assert.equal(slot.active, hunks[1], "Shift+Tab from first hunk must wrap to last");
  // Enter on the first hunk should trigger the accept action. We can't actually
  // invoke host persistence in a unit test, so we capture the click on the
  // accept button and verify the handler is wired up.
  slot.active = hunks[0];
  const acceptButton = hunks[0].querySelector(".hunk-actions button.small-button");
  assert.ok(acceptButton, "hunk must render an accept button");
  let clicked = false;
  acceptButton.click = () => { clicked = true; };
  const enter = makeKeyEvent(doc, "Enter");
  hunks[0].dispatchEvent(enter);
  assert.ok(enter.defaultPrevented, "Enter on hunk must be preventDefault'd");
  assert.ok(clicked, "Enter on hunk must click the accept button");
  // Shift+Enter on the second hunk triggers the reject button.
  slot.active = hunks[1];
  const rejectButton = hunks[1].querySelector(".hunk-actions button.quiet-button");
  assert.ok(rejectButton, "hunk must render a reject button");
  let rejected = false;
  rejectButton.click = () => { rejected = true; };
  const shiftEnter = makeKeyEvent(doc, "Enter", { shiftKey: true });
  hunks[1].dispatchEvent(shiftEnter);
  assert.ok(shiftEnter.defaultPrevented, "Shift+Enter on hunk must be preventDefault'd");
  assert.ok(rejected, "Shift+Enter on hunk must click the reject button");
});

// T09: patch review state changes are announced via announceLive.
test("T09: announceReviewState updates the aria-live region with the current state", async () => {
  resetReviewState();
  // Clear any pre-existing live region so we can assert it was created fresh.
  const liveBefore = document.querySelectorAll("#optimizer-a11y-live");
  for (const el of liveBefore) el.remove();

  state.aiReview = {
    kind: "patch_proposal",
    busy: false,
    session: {
      revision: 1,
      status: "review",
      decisions: { "hunk-1": "accepted" },
    },
    result: {
      kind: "patch_proposal",
      proposal: {
        id: "prop-1",
        summary: "test",
        hunks: [
          { id: "hunk-1", granularity: "block", original: "old", replacement: "new" },
          { id: "hunk-2", granularity: "block", original: "old2", replacement: "new2" },
        ],
      },
    },
  };
  announceReviewState();
  // announceLive uses a microtask to update textContent; flush.
  await Promise.resolve();
  await Promise.resolve();
  const live = document.getElementById("optimizer-a11y-live");
  assert.ok(live, "announceReviewState must populate the live region");
  const message = live.textContent || "";
  assert.ok(message.length > 0, "announcement must be non-empty");
  assert.ok(
    message.includes("1") || message.includes("接受") || message.includes("accept") || message.includes("review"),
    `announcement must reference the accepted count or review state, got: ${message}`,
  );
});
