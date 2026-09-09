import assert from "node:assert/strict";
import test from "node:test";
import { parseHTML } from "linkedom";
import { pathToFileURL } from "node:url";
import { resolve, dirname } from "node:path";
import { fileURLToPath } from "node:url";

// v0.9.0 Stage 4 (D-4): review UX hardening.
//   T01 — j/k move focus between hunks (forward/back) and stay at boundaries
//   T02 — bulk warning renders only when hunk count > 12
//   T03 — j/k ignore modifier combos and still work when no hunk is focused
//   T04 — review.bulkWarning is defined in both zh-CN and en-US (no emoji)
//
// linkedom does not implement focus()/document.activeElement like a real
// browser, so — following the R-07 lesson from a11y-dom.test.js — we install
// an activeElement stub and patch focus()/blur() on the rendered tree so the
// keyboard handler can read and move focus deterministically.

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

const { state, aiReviewView } = await import("../dist/main.js");

const __dirname = dirname(fileURLToPath(import.meta.url));
const srcDir = resolve(__dirname, "..", "src");

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

function makeKeyEvent(doc, key, { shiftKey = false, ctrlKey = false, metaKey = false, altKey = false } = {}) {
  const event = new doc.defaultView.Event("keydown", {
    bubbles: true,
    cancelable: true,
  });
  Object.defineProperty(event, "key", { value: key, configurable: true });
  Object.defineProperty(event, "shiftKey", { value: shiftKey, configurable: true });
  Object.defineProperty(event, "ctrlKey", { value: ctrlKey, configurable: true });
  Object.defineProperty(event, "metaKey", { value: metaKey, configurable: true });
  Object.defineProperty(event, "altKey", { value: altKey, configurable: true });
  event.preventDefault = function preventDefault() {
    Object.defineProperty(this, "defaultPrevented", {
      value: true,
      configurable: true,
    });
  };
  return event;
}

function makeHunks(n) {
  return Array.from({ length: n }, (_, i) => ({
    id: `hunk-${i + 1}`,
    granularity: "block",
    original: `old-${i + 1}`,
    replacement: `new-${i + 1}`,
  }));
}

function setReviewState(hunkCount, { candidateBranch = null, summary = "test" } = {}) {
  state.aiReview = {
    kind: "patch_proposal",
    busy: false,
    session: { revision: 1, status: "review", decisions: {} },
    result: {
      kind: "patch_proposal",
      proposal: {
        id: "prop-1",
        summary,
        hunks: makeHunks(hunkCount),
      },
    },
    candidateBranch,
  };
  state.workspace = { documents: [{ id: "doc-1", title: "Doc 1" }], blocks: [] };
  state.aiRunning = null;
  state.candidateBusy = null;
}

// T01: j/k move focus between hunks; at the boundaries focus stays put.
test("T01: j/k move focus between hunks and stay at first/last boundary", () => {
  const { slot, patchAll } = installActiveElementStub(global.document);
  setReviewState(4);
  const view = aiReviewView(global.document);
  assert.ok(view, "aiReviewView must render a section");
  patchAll(view);
  const hunks = view.querySelectorAll(".review-hunk");
  assert.equal(hunks.length, 4, "must render 4 hunks");

  // From hunk #2 (index 1), j moves focus to hunk #3.
  slot.active = hunks[1];
  let j = makeKeyEvent(global.document, "j");
  view.dispatchEvent(j);
  assert.ok(j.defaultPrevented, "j on a middle hunk must be preventDefault'd");
  assert.equal(slot.active, hunks[2], "j must move focus to the next hunk");

  // From hunk #3 (index 2), k moves focus back to hunk #2.
  slot.active = hunks[2];
  let k = makeKeyEvent(global.document, "k");
  view.dispatchEvent(k);
  assert.ok(k.defaultPrevented, "k on a middle hunk must be preventDefault'd");
  assert.equal(slot.active, hunks[1], "k must move focus to the previous hunk");

  // At the last hunk, j stays put (no wrap).
  slot.active = hunks[3];
  let jLast = makeKeyEvent(global.document, "j");
  view.dispatchEvent(jLast);
  assert.ok(!jLast.defaultPrevented, "j at the last hunk must not intercept (boundary stay)");
  assert.equal(slot.active, hunks[3], "j at the last hunk must keep focus on the last hunk");

  // At the first hunk, k stays put (no wrap).
  slot.active = hunks[0];
  let kFirst = makeKeyEvent(global.document, "k");
  view.dispatchEvent(kFirst);
  assert.ok(!kFirst.defaultPrevented, "k at the first hunk must not intercept (boundary stay)");
  assert.equal(slot.active, hunks[0], "k at the first hunk must keep focus on the first hunk");
});

// T02: bulk warning bar renders only when hunk count > 12.
test("T02: bulk warning renders when hunks > 12 and is absent at <= 12", () => {
  // Exactly 12 hunks -> no warning bar.
  setReviewState(12);
  const view12 = aiReviewView(global.document);
  assert.ok(view12, "aiReviewView must render a section for 12 hunks");
  assert.equal(
    view12.querySelectorAll(".review-bulk-warning").length,
    0,
    "12 hunks must NOT render the bulk warning bar",
  );

  // 13 hunks -> warning bar present with role=status and the count.
  setReviewState(13);
  const view13 = aiReviewView(global.document);
  assert.ok(view13, "aiReviewView must render a section for 13 hunks");
  const warnings = view13.querySelectorAll(".review-bulk-warning");
  assert.equal(warnings.length, 1, "13 hunks must render exactly one bulk warning bar");
  const warning = warnings[0];
  assert.equal(
    warning.getAttribute("role"),
    "status",
    "bulk warning bar must carry role=status",
  );
  assert.ok(
    (warning.textContent || "").includes("13"),
    "bulk warning bar must mention the hunk count (13)",
  );
});

// T03: j/k ignore modifier combos, and work when no hunk is focused yet.
test("T03: j/k ignore modifier combos and focus first/last when nothing is focused", () => {
  const { slot, patchAll } = installActiveElementStub(global.document);
  setReviewState(3);
  const view = aiReviewView(global.document);
  patchAll(view);
  const hunks = view.querySelectorAll(".review-hunk");
  assert.equal(hunks.length, 3, "must render 3 hunks");

  // Ctrl+j must be ignored (left to the browser / host shortcuts).
  slot.active = hunks[0];
  let ctrlJ = makeKeyEvent(global.document, "j", { ctrlKey: true });
  view.dispatchEvent(ctrlJ);
  assert.ok(!ctrlJ.defaultPrevented, "Ctrl+j must not be intercepted by the review handler");
  assert.equal(slot.active, hunks[0], "Ctrl+j must not move focus");

  // Alt+k must be ignored too.
  slot.active = hunks[1];
  let altK = makeKeyEvent(global.document, "k", { altKey: true });
  view.dispatchEvent(altK);
  assert.ok(!altK.defaultPrevented, "Alt+k must not be intercepted by the review handler");
  assert.equal(slot.active, hunks[1], "Alt+k must not move focus");

  // When nothing is focused (activeElement outside a hunk), j focuses the
  // first hunk and k focuses the last hunk.
  slot.active = null;
  let jFromNone = makeKeyEvent(global.document, "j");
  view.dispatchEvent(jFromNone);
  assert.ok(jFromNone.defaultPrevented, "j with no focused hunk must be preventDefault'd");
  assert.equal(slot.active, hunks[0], "j with no focused hunk must focus the first hunk");

  slot.active = null;
  let kFromNone = makeKeyEvent(global.document, "k");
  view.dispatchEvent(kFromNone);
  assert.ok(kFromNone.defaultPrevented, "k with no focused hunk must be preventDefault'd");
  assert.equal(slot.active, hunks[2], "k with no focused hunk must focus the last hunk");
});

// T04: review.bulkWarning exists in both locales, is non-empty, and has no emoji.
test("T04: review.bulkWarning is defined in zh-CN and en-US without emoji", async () => {
  const { default: zh } = await import(pathToFileURL(resolve(srcDir, "locales/zh-CN.js")).href);
  const { default: en } = await import(pathToFileURL(resolve(srcDir, "locales/en-US.js")).href);
  for (const [locale, dict] of [["zh-CN", zh], ["en-US", en]]) {
    assert.ok(
      Object.prototype.hasOwnProperty.call(dict, "review.bulkWarning"),
      `${locale} must define review.bulkWarning`,
    );
    assert.ok(
      typeof dict["review.bulkWarning"] === "string" && dict["review.bulkWarning"].length > 0,
      `${locale} review.bulkWarning must be a non-empty string`,
    );
    const emojiPattern = /[\u{1F300}-\u{1FAFF}\u{2600}-\u{27BF}]/u;
    assert.ok(
      !emojiPattern.test(dict["review.bulkWarning"]),
      `${locale} review.bulkWarning must not contain emoji`,
    );
  }
  // Interpolated count must appear in the rendered zh-CN message.
  assert.ok(
    zh["review.bulkWarning"].includes("{count}"),
    "zh-CN review.bulkWarning must accept a {count} placeholder",
  );
  // Same structural guarantee for en-US: the interpolated count placeholder
  // must exist so the rendered English warning shows the hunk count too.
  assert.ok(
    en["review.bulkWarning"].includes("{count}"),
    "en-US review.bulkWarning must accept a {count} placeholder",
  );
});
