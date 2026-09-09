// scripts/check-motion-tokens.mjs
// 守门脚本：验证 styles.css 的 design tokens 完整性。
// FAIL = 必需 token/预设类缺失（exit 1）；WARN = 仍存在的字面量时长/缓动（不阻断，仅提示）。
import { readFile } from "node:fs/promises";
import { resolve, dirname } from "node:path";
import { fileURLToPath } from "node:url";

const root = resolve(dirname(fileURLToPath(import.meta.url)), "..");
const cssPath = resolve(root, "apps/optimizer-desktop/src/styles.css");

const REQUIRED_TOKENS = [
  "--space-xxs: 2px",
  "--space-xs: 4px",
  "--space-sm: 8px",
  "--space-md: 16px",
  "--space-lg: 24px",
  "--space-xl: 40px",
  "--space-2xl: 48px",
  "--space-3xl: 56px",
  "--space-4xl: 64px",
  "--container-prose: 680px",
  "--container-content: 1120px",
  "--container-dialog: 600px",
  "--radius-pill: 999px",
  "--motion-instant: 80ms",
  "--motion-quick: 150ms",
  "--motion-standard: 240ms",
  "--motion-deliberate: 360ms",
  "--motion-entrance: 520ms",
  "--motion-progress: 1.4s",
  "--motion-pulse: 1.6s",
  "--motion-ambient: 5s",
  "--ease-standard:",
  "--ease-emphasized:",
  "--ease-linear:",
];

const REQUIRED_PRESETS = [".motion-fade", ".motion-lift", ".motion-slide", ".motion-scale"];

const css = await readFile(cssPath, "utf8");
const missing = REQUIRED_TOKENS.filter((token) => !css.includes(token));
const missingPresets = REQUIRED_PRESETS.filter((cls) => !css.includes(cls));

const hardcodedTimings = [];
const timingRegex = /(?:transition|animation)(?:-[a-z]+)?\s*:\s*[^;{}]*?(\d+(?:\.\d+)?(?:ms|s))[^;{}]*;/gi;
let match;
while ((match = timingRegex.exec(css)) !== null) {
  if (!match[1].includes("var(")) hardcodedTimings.push(match[0].trim().slice(0, 90));
}
const hardcodedEasing = [];
const easingRegex = /(?:transition|animation)(?:-[a-z]+)?\s*:\s*[^;{}]*?(ease-in-out|ease-in|ease-out|ease(?![-\w])|cubic-bezier\()/gi;
while ((match = easingRegex.exec(css)) !== null) {
  if (!/var\(--ease/.test(match[0])) hardcodedEasing.push(match[0].trim().slice(0, 90));
}

let failed = false;
if (missing.length) {
  failed = true;
  console.error("FAIL: 缺少必需 token ——");
  for (const token of missing) console.error(`  ✗ ${token}`);
} else {
  console.log("PASS: 9 级间距 / 容器 / radius-pill token 齐备");
}
if (missingPresets.length) {
  failed = true;
  console.error("FAIL: 缺少预设类 ——");
  for (const preset of missingPresets) console.error(`  ✗ ${preset}`);
} else {
  console.log("PASS: .motion-fade / .motion-lift / .motion-slide / .motion-scale 齐备");
}
if (hardcodedTimings.length) console.warn(`WARN: ${hardcodedTimings.length} 处字面量时长（建议改用 var(--motion-*)）`);
if (hardcodedEasing.length) console.warn(`WARN: ${hardcodedEasing.length} 处字面量缓动（建议改用 var(--ease-*)）`);

if (failed) {
  console.error("\n结果: FAIL");
  process.exit(1);
}
console.log("\n结果: PASS");
