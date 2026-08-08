import assert from "node:assert/strict";
import test from "node:test";
import { parseHTML } from "linkedom";
import { readFile } from "node:fs/promises";
import { resolve, dirname } from "node:path";
import { fileURLToPath, pathToFileURL } from "node:url";

// v0.8.0 Stage 3 (a11y): unit tests for the pure ES Module a11y helpers.
// These tests cover the lowest-level contracts:
//   T01 — trapFocus cycles Tab/Shift+Tab within the container (no escape)
//   T02 — trapFocus release() restores focus to the opener element
//   T03 — announceLive creates/updates an aria-live=polite div
//   T10 — styles.css declares :focus-visible and never uses outline: none
//   T11 — a11y.* i18n keys cover every icon-button aria-label we render
//
// linkedom does not implement focus() / document.activeElement the way real
// browsers do. To keep these tests deterministic we install a stub that
// turns focus() into a setter on a shared `state.active` slot, and we make
// `document.activeElement` return that slot. The trapFocus module reads
// `document.activeElement` and calls `el.focus()` so this is sufficient.

const bootstrapDom = parseHTML(`<!DOCTYPE html><html><body></body></html>`);
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

const __dirname = dirname(fileURLToPath(import.meta.url));
const srcDir = resolve(__dirname, "..", "src");

const { trapFocus, announceLive, _resetLiveElementForTests } = await import(
  "../dist/a11y.js"
);

function setupDoc() {
  const dom = parseHTML(`<!DOCTYPE html><html><body></body></html>`);
  const document = dom.document;
  const slot = { active: null };
  // Override activeElement getter so trapFocus can read the "current" focus.
  Object.defineProperty(document, "activeElement", {
    configurable: true,
    get() {
      return slot.active;
    },
  });
  // Patch every element's focus()/blur() so they update the slot. We patch
  // on demand (called after the test assembles its tree) because linkedom
  // elements do not have a working focus() by default.
  const patch = (el) => {
    el.focus = function focus() { slot.active = el; };
    el.blur = function blur() { if (slot.active === el) slot.active = null; };
    return el;
  };
  const patchAll = (root) => {
    if (!root || !root.querySelectorAll) return;
    for (const el of root.querySelectorAll("*")) patch(el);
    if (root.focus !== patch) patch(root);
  };
  return { document, slot, patch, patchAll };
}

// linkedom's defaultView does not expose a KeyboardEvent constructor. We
// build the event with the standard Event constructor and define key /
// shiftKey on it so trapFocus's keydown handler can read them. preventDefault
// is overridden so the test can observe whether the handler called it.
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

// T01: trapFocus cycles Tab within the container (no escape).
test("T01: trapFocus cycles Tab/Shift+Tab within the container", () => {
  const { document, slot, patchAll } = setupDoc();
  const container = document.createElement("div");
  const btn1 = document.createElement("button");
  btn1.textContent = "First";
  const btn2 = document.createElement("button");
  btn2.textContent = "Second";
  const btn3 = document.createElement("button");
  btn3.textContent = "Third";
  container.append(btn1, btn2, btn3);
  document.body.append(container);
  patchAll(container);
  // Patch body too so document.activeElement defaults are stable.
  patchAll(document.body);

  const release = trapFocus(container, { document });
  assert.equal(typeof release, "function", "trapFocus must return a release fn");

  // Initial focus lands on the first focusable element.
  assert.equal(slot.active, btn1, "trapFocus must move focus to the first item");

  // Forward Tab from the last item wraps to the first.
  slot.active = btn3;
  let forward = makeKeyEvent(document, "Tab");
  container.dispatchEvent(forward);
  assert.ok(forward.defaultPrevented, "Tab on last item must be prevented");
  assert.equal(slot.active, btn1, "Tab from last must wrap to first");

  // Forward Tab from a middle item is left to the browser (no preventDefault).
  slot.active = btn2;
  let mid = makeKeyEvent(document, "Tab");
  container.dispatchEvent(mid);
  assert.ok(!mid.defaultPrevented, "Tab on middle item must not be intercepted");

  // Shift+Tab from the first item wraps to the last.
  slot.active = btn1;
  let backward = makeKeyEvent(document, "Tab", { shiftKey: true });
  container.dispatchEvent(backward);
  assert.ok(backward.defaultPrevented, "Shift+Tab on first item must be prevented");
  assert.equal(slot.active, btn3, "Shift+Tab from first must wrap to last");

  // Non-Tab keys are ignored entirely.
  slot.active = btn1;
  let escape = makeKeyEvent(document, "Escape");
  container.dispatchEvent(escape);
  assert.ok(!escape.defaultPrevented, "Escape must not be intercepted by trapFocus");

  release();
});

// T02: trapFocus release() returns focus to the opener element.
test("T02: trapFocus release() restores focus to the opener", () => {
  const { document, slot, patchAll } = setupDoc();
  const opener = document.createElement("button");
  opener.textContent = "Opener";
  const container = document.createElement("div");
  const inner = document.createElement("button");
  inner.textContent = "Inner";
  container.append(inner);
  document.body.append(opener, container);
  patchAll(document.body);

  // Simulate the user clicking the opener before the drawer opens.
  slot.active = opener;
  const release = trapFocus(container, { document });
  assert.equal(slot.active, inner, "trapFocus must move focus into the container");

  release();
  assert.equal(slot.active, opener, "release() must restore focus to the opener");

  // After release, keydown listeners are detached — Tab no longer wraps.
  slot.active = inner;
  const tab = makeKeyEvent(document, "Tab");
  container.dispatchEvent(tab);
  assert.ok(!tab.defaultPrevented, "release() must detach the Tab listener");
});

// T02 (cont.): trapFocus is a no-op when document is unavailable (defensive).
test("T02: trapFocus returns a no-op release when document is missing", () => {
  const container = { addEventListener: () => {} };
  const release = trapFocus(container, { document: null });
  assert.equal(typeof release, "function");
  release(); // must not throw
});

// T03: announceLive creates an aria-live=polite div and announces messages.
test("T03: announceLive creates aria-live=polite div and announces the message", async () => {
  _resetLiveElementForTests();
  const { document } = setupDoc();
  // linkedom's document.body exists; announceLive must append to it.
  assert.ok(document.body, "precondition: document.body must exist");

  announceLive("项目已恢复", true, { document });
  // announceLive uses a microtask to set textContent so screen readers
  // observe the change from empty -> message. Flush microtasks.
  await Promise.resolve();
  await Promise.resolve();

  const live = document.getElementById("optimizer-a11y-live");
  assert.ok(live, "announceLive must create #optimizer-a11y-live element");
  assert.equal(
    live.getAttribute("aria-live"),
    "polite",
    "default channel must be aria-live=polite",
  );
  assert.equal(
    live.getAttribute("aria-atomic"),
    "true",
    "aria-atomic=true so the whole message is read",
  );
  assert.equal(
    live.getAttribute("role"),
    "status",
    "role=status so SR treat it as a live region",
  );
  assert.equal(live.textContent, "项目已恢复", "message must be announced");

  // Subsequent calls reuse the same element and update text.
  announceLive("已应用 Patch", true, { document });
  await Promise.resolve();
  await Promise.resolve();
  const same = document.getElementById("optimizer-a11y-live");
  assert.equal(same, live, "subsequent announceLive must reuse the existing element");
  assert.equal(same.textContent, "已应用 Patch", "message must be updated");
});

// T03 (cont.): announceLive(polite=false) switches to aria-live=assertive.
test("T03: announceLive(polite=false) uses aria-live=assertive", async () => {
  _resetLiveElementForTests();
  const { document } = setupDoc();
  announceLive("严重错误", false, { document });
  await Promise.resolve();
  await Promise.resolve();
  const live = document.getElementById("optimizer-a11y-live");
  assert.equal(
    live.getAttribute("aria-live"),
    "assertive",
    "polite=false must produce aria-live=assertive",
  );
});

// T03 (cont.): announceLive reuses an existing live element across calls.
test("T03: announceLive reuses one live element across multiple calls", async () => {
  _resetLiveElementForTests();
  const { document } = setupDoc();
  announceLive("first", true, { document });
  await Promise.resolve();
  await Promise.resolve();
  announceLive("second", true, { document });
  await Promise.resolve();
  await Promise.resolve();
  announceLive("third", true, { document });
  await Promise.resolve();
  await Promise.resolve();
  const all = document.querySelectorAll("#optimizer-a11y-live");
  assert.equal(all.length, 1, "only one #optimizer-a11y-live element must exist");
  assert.equal(all[0].textContent, "third", "latest message must be present");
});

// T10: :focus-visible style exists in styles.css and never removes outline.
test("T10: styles.css declares :focus-visible and never uses outline: none", async () => {
  const css = await readFile(resolve(srcDir, "styles.css"), "utf8");
  assert.ok(
    /:focus-visible\s*\{/.test(css),
    "styles.css must declare a :focus-visible rule",
  );
  assert.ok(
    /outline:\s*[^;]*#3b82f6/i.test(css) || /outline:\s*2px/i.test(css),
    ":focus-visible must set an outline (2px solid #3b82f6 by spec)",
  );
  // No bare `outline: none` declarations are allowed anywhere — they would
  // defeat keyboard-only focus visibility. We accept `outline: none` only
  // when immediately followed by a `:focus-visible` override on the same
  // selector, but for safety we just forbid the bare form.
  assert.ok(
    !/outline:\s*none\s*[;}\n]/i.test(css),
    "styles.css must not contain a bare `outline: none` declaration",
  );
});

// T11: a11y.* i18n keys exist in both zh-CN and en-US locales and cover the
// drawer close buttons, command palette close, drag handle, and hunk group.
test("T11: a11y.* keys exist in zh-CN and en-US and cover icon button aria-labels", async () => {
  const { default: zh } = await import(pathToFileURL(resolve(srcDir, "locales/zh-CN.js")).href);
  const { default: en } = await import(pathToFileURL(resolve(srcDir, "locales/en-US.js")).href);
  const requiredKeys = [
    "a11y.closeDrawer",
    "a11y.closeCommandPalette",
    "a11y.closeReview",
    "a11y.dragHandle",
    "a11y.hunkGroup",
    "a11y.commandPaletteList",
    "a11y.drawerDialog",
    "a11y.reviewDialog",
    "a11y.commandPaletteDialog",
    "a11y.hunkAccept",
    "a11y.hunkReject",
    "a11y.liveRegionLabel",
  ];
  for (const key of requiredKeys) {
    assert.ok(
      Object.prototype.hasOwnProperty.call(zh, key),
      `zh-CN must define ${key}`,
    );
    assert.ok(
      Object.prototype.hasOwnProperty.call(en, key),
      `en-US must define ${key}`,
    );
    assert.ok(
      typeof zh[key] === "string" && zh[key].length > 0,
      `zh-CN value for ${key} must be a non-empty string`,
    );
    assert.ok(
      typeof en[key] === "string" && en[key].length > 0,
      `en-US value for ${key} must be a non-empty string`,
    );
    // UI must not contain emoji (user preference).
    const emojiPattern = /[\u{1F300}-\u{1FAFF}\u{2600}-\u{27BF}]/u;
    assert.ok(
      !emojiPattern.test(zh[key]),
      `zh-CN ${key} must not contain emoji`,
    );
    assert.ok(
      !emojiPattern.test(en[key]),
      `en-US ${key} must not contain emoji`,
    );
  }
});
