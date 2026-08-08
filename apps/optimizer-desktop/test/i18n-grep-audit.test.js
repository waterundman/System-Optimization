import assert from "node:assert/strict";
import test from "node:test";
import { readFile } from "node:fs/promises";
import { resolve, dirname } from "node:path";
import { fileURLToPath } from "node:url";

// v0.8.0 Stage 2: i18n grep audit.
// Verifies that apps/optimizer-desktop/src/main.js contains no hardcoded
// Chinese strings in string literals. After migration, all user-facing
// Chinese text must live in locales/zh-CN.json and be accessed via t().
// Chinese characters inside comments (// or /* */) are permitted.

const __dirname = dirname(fileURLToPath(import.meta.url));
const sourcePath = resolve(__dirname, "..", "src", "main.js");

// Match string literals delimited by ", ', or `. Uses non-greedy
// matching with escape support. [\s\S] allows newlines inside template
// literals. The (?!\1) lookahead ensures we don't consume the closing
// quote as a content character.
const STRING_PATTERN = /(["'`])(?:\\.|(?!\1)[\s\S])*?\1/g;
const CJK_PATTERN = /[\u4e00-\u9fff]/;

// Strip block comments (/* ... */) and line comments (// ...) so Chinese
// text inside comments does not trigger false positives. This is a
// conservative stripper: it may remove // inside strings, but those
// occurrences do not contain CJK characters in the current codebase.
function stripComments(source) {
  return source
    .replace(/\/\*[\s\S]*?\*\//g, "")
    .replace(/\/\/[^\n]*/g, "");
}

function findChineseStrings(source) {
  const stripped = stripComments(source);
  const matches = [...stripped.matchAll(STRING_PATTERN)];
  const violations = [];
  for (const match of matches) {
    const text = match[0];
    if (CJK_PATTERN.test(text)) {
      // Truncate long strings for readability in the error message.
      const preview = text.length > 100 ? text.slice(0, 100) + "…" : text;
      violations.push(preview);
    }
  }
  return violations;
}

test("T26: main.js source contains no hardcoded Chinese string literals", async () => {
  const source = await readFile(sourcePath, "utf8");
  const violations = findChineseStrings(source);
  assert.equal(
    violations.length,
    0,
    `Found ${violations.length} hardcoded Chinese string(s) in src/main.js. ` +
      `All user-facing text must be migrated to locales/zh-CN.json and accessed via t().\n` +
      `Violations:\n${violations.join("\n")}`,
  );
});

test("T27: main.js imports the i18n module", async () => {
  const source = await readFile(sourcePath, "utf8");
  assert.ok(
    /import\s+\{[^}]*\bt\b[^}]*\}\s+from\s+["']\.\/i18n\.js["']/.test(source),
    "main.js must import t() from ./i18n.js",
  );
});

test("T28: main.js state object includes a locale field", async () => {
  const source = await readFile(sourcePath, "utf8");
  assert.ok(
    /locale:\s*["']zh-CN["']/.test(source),
    "main.js state object must include a locale field initialized to 'zh-CN'",
  );
});
