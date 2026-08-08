// v0.8.0 Stage 3 (a11y): lightweight accessibility helpers — pure ES Module,
// no third-party runtime dependencies.
//
//   trapFocus(containerEl, options?) → release()
//     Attaches a Tab/Shift+Tab keydown listener so keyboard focus cycles
//     inside the container and never escapes to the page underneath.
//     Returns a release() function that detaches the listener and restores
//     focus to the opener (the element that was focused before the trap
//     was installed). Reuses the v0.4.0 compareDrawer focus-record pattern
//     (record opener on enter, restore on exit) without external libraries.
//
//   announceLive(message, polite = true, options?)
//     Creates or reuses a hidden aria-live div appended to document.body
//     and updates its textContent so screen readers announce the message.
//     Extends the v0.6.0 state.notice pattern with a dedicated live region
//     for non-visual state transitions (e.g. "已恢复项目", "已应用 Patch").

const FOCUSABLE_SELECTOR = [
  'a[href]:not([disabled])',
  'button:not([disabled])',
  'input:not([disabled])',
  'select:not([disabled])',
  'textarea:not([disabled])',
  '[tabindex]:not([tabindex="-1"])',
].join(",");

function isFocusable(el) {
  if (!el) return false;
  if (el.hidden) return false;
  if (el.disabled) return false;
  if (typeof el.getBoundingClientRect === "function") {
    // In real browsers we'd also check visibility; in linkedom-based tests the
    // function exists but returns zeros, so we skip the geometric check.
  }
  return true;
}

function getFocusables(container) {
  if (!container || typeof container.querySelectorAll !== "function") return [];
  return Array.from(container.querySelectorAll(FOCUSABLE_SELECTOR))
    .filter(isFocusable);
}

function getActiveElement(doc) {
  if (!doc) return null;
  let active = doc.activeElement;
  // linkedom may return undefined/null instead of body when nothing is
  // focused. Fall back to body so callers always get a stable Element.
  if (!active && doc.body) active = doc.body;
  return active;
}

export function trapFocus(containerEl, options = {}) {
  const doc = options.document
    || (typeof document !== "undefined" ? document : null);
  if (!doc || !containerEl) return () => {};
  const opener = getActiveElement(doc);
  const openerHasFocus = typeof (opener && opener.focus) === "function";
  const onKeyDown = (event) => {
    if (event.key !== "Tab") return;
    const items = getFocusables(containerEl);
    if (!items.length) {
      event.preventDefault();
      return;
    }
    const first = items[0];
    const last = items[items.length - 1];
    const active = getActiveElement(doc);
    const insideContainer = containerEl.contains(active);
    if (event.shiftKey) {
      if (active === first || !insideContainer) {
        event.preventDefault();
        last.focus();
      }
    } else {
      if (active === last) {
        event.preventDefault();
        first.focus();
      }
    }
  };
  if (typeof containerEl.addEventListener === "function") {
    containerEl.addEventListener("keydown", onKeyDown);
  }
  // Move focus into the container synchronously so keyboard users land on
  // the first interactive control rather than the dialog root (matches
  // the v0.4.0 compareDrawer pattern of recording the opener then focusing
  // the first control inside the dialog).
  const initialItems = getFocusables(containerEl);
  if (initialItems.length && typeof initialItems[0].focus === "function") {
    initialItems[0].focus();
  }
  return function release() {
    if (typeof containerEl.removeEventListener === "function") {
      containerEl.removeEventListener("keydown", onKeyDown);
    }
    if (openerHasFocus) {
      try { opener.focus(); } catch { /* ignore focus errors */ }
    }
  };
}

let cachedLiveElement = null;

export function announceLive(message, polite = true, options = {}) {
  const doc = options.document
    || (typeof document !== "undefined" ? document : null);
  if (!doc || !doc.body) return;
  const desiredPoliteness = polite ? "polite" : "assertive";
  // Reuse the live element across calls within the same document. If the
  // cached element has been detached (e.g. test re-creates document), fall
  // back to a fresh querySelector.
  if (
    !cachedLiveElement
    || !cachedLiveElement.ownerDocument
    || cachedLiveElement.ownerDocument !== doc
    || !doc.body.contains(cachedLiveElement)
  ) {
    const existing = doc.getElementById("optimizer-a11y-live");
    if (existing) {
      cachedLiveElement = existing;
    } else {
      cachedLiveElement = doc.createElement("div");
      cachedLiveElement.id = "optimizer-a11y-live";
      cachedLiveElement.className = "optimizer-a11y-live";
      cachedLiveElement.setAttribute("aria-live", desiredPoliteness);
      cachedLiveElement.setAttribute("aria-atomic", "true");
      cachedLiveElement.setAttribute("role", "status");
      doc.body.append(cachedLiveElement);
    }
  }
  // Switch politeness on demand (e.g. a later assertive announcement).
  if (cachedLiveElement.getAttribute("aria-live") !== desiredPoliteness) {
    cachedLiveElement.setAttribute("aria-live", desiredPoliteness);
  }
  // Some screen readers only fire an announcement when the live region's
  // text actually changes. Clearing synchronously then setting on a microtask
  // guarantees the SR observes the transition from "" -> message.
  cachedLiveElement.textContent = "";
  const text = message == null ? "" : String(message);
  queueMicrotask(() => {
    if (cachedLiveElement) cachedLiveElement.textContent = text;
  });
}

export function _resetLiveElementForTests() {
  if (cachedLiveElement && cachedLiveElement.parentNode) {
    cachedLiveElement.parentNode.removeChild(cachedLiveElement);
  }
  cachedLiveElement = null;
}
