// v0.9.0: 统一动画调度模块 — 预设类映射 / applyMotion / staggerEnter / withReducedMotion。
// 与 styles.css 中的 --motion-* / --ease-* token 及 .motion-* 预设类对应。
// 接入点（后续迭代）：welcome.js 入场、workspace.js 抽屉切换。纯 ES Module，无第三方依赖。

export const MOTION_PRESETS = Object.freeze({
  fade: { animationName: "fadeIn", duration: "var(--motion-standard)", easing: "var(--ease-standard)", fill: "both" },
  lift: { animationName: "fadeUp", duration: "var(--motion-entrance)", easing: "var(--ease-emphasized)", fill: "both" },
  slide: { animationName: "slideInRight", duration: "var(--motion-deliberate)", easing: "var(--ease-emphasized)", fill: "both" },
  scale: { animationName: "scaleIn", duration: "var(--motion-standard)", easing: "var(--ease-emphasized)", fill: "both" },
});

const reducedMotionQuery =
  typeof window !== "undefined" && typeof window.matchMedia === "function"
    ? window.matchMedia("(prefers-reduced-motion: reduce)")
    : null;

export function prefersReducedMotion() {
  return Boolean(reducedMotionQuery && reducedMotionQuery.matches);
}

export function withReducedMotion(fn) {
  if (prefersReducedMotion()) return fn();
  return undefined;
}

export function applyMotion(element, presetName, { delay = 0 } = {}) {
  if (!element || typeof element.style !== "object") return;
  const preset = MOTION_PRESETS[presetName] ?? MOTION_PRESETS.fade;
  element.style.animationName = preset.animationName;
  element.style.animationDuration = preset.duration;
  element.style.animationTimingFunction = preset.easing;
  element.style.animationFillMode = preset.fill;
  if (delay > 0) element.style.animationDelay = `${delay}ms`;
}

export function staggerEnter(elements, { preset = "fade", delay = 80 } = {}) {
  if (!Array.isArray(elements) && typeof elements?.[Symbol.iterator] === "function") elements = Array.from(elements);
  if (!Array.isArray(elements)) return;
  if (prefersReducedMotion() || elements.length === 0) return;
  elements.forEach((el, index) => {
    if (!el || typeof el.style !== "object") return;
    requestAnimationFrame(() => applyMotion(el, preset, { delay: index * delay }));
  });
}
