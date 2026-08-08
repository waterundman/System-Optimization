import { readFile } from "node:fs/promises";
import { resolve, dirname } from "node:path";
import { fileURLToPath, pathToFileURL } from "node:url";

// v0.8.0 Stage 2: CI validation for the i18n dictionary layer.
// Verifies that zh-CN and en-US locale files have identical key sets,
// that the i18n module exports the required functions, and that no
// translation value is empty. This keeps the fallback chain honest:
// a missing key in en-US would silently fall through to zh-CN (or the
// raw key), which is acceptable at runtime but must be caught in CI so
// translators know which keys still need coverage.
//
// Locale dictionaries are plain ES modules (not JSON) so the desktop
// runtime never fetches them through the CSP-controlled asset protocol.

const root = resolve(dirname(fileURLToPath(import.meta.url)), "..");
const localesDir = resolve(root, "apps/optimizer-desktop/src/locales");
const i18nModule = resolve(root, "apps/optimizer-desktop/src/i18n.js");

const { default: zhCN } = await import(pathToFileURL(resolve(localesDir, "zh-CN.js")).href);
const { default: enUS } = await import(pathToFileURL(resolve(localesDir, "en-US.js")).href);

function collectKeys(value, prefix = "", output = new Set()) {
  if (value && typeof value === "object" && !Array.isArray(value)) {
    for (const [key, child] of Object.entries(value)) {
      const path = prefix ? `${prefix}.${key}` : key;
      collectKeys(child, path, output);
    }
  } else {
    output.add(prefix);
  }
  return output;
}

const zhKeys = collectKeys(zhCN);
const enKeys = collectKeys(enUS);

const missingInEn = [...zhKeys].filter((key) => !enKeys.has(key));
const missingInZh = [...enKeys].filter((key) => !zhKeys.has(key));

if (missingInEn.length > 0) {
  throw new Error(
    `i18n key parity failed: ${missingInEn.length} key(s) present in zh-CN but missing in en-US:\n` +
      missingInEn.map((key) => `  - ${key}`).join("\n"),
  );
}

if (missingInZh.length > 0) {
  throw new Error(
    `i18n key parity failed: ${missingInZh.length} key(s) present in en-US but missing in zh-CN:\n` +
      missingInZh.map((key) => `  - ${key}`).join("\n"),
  );
}

function collectEmptyKeys(value, prefix = "", output = []) {
  if (value && typeof value === "object" && !Array.isArray(value)) {
    for (const [key, child] of Object.entries(value)) {
      const path = prefix ? `${prefix}.${key}` : key;
      collectEmptyKeys(child, path, output);
    }
  } else if (typeof value !== "string" || value.length === 0) {
    output.push(prefix);
  }
  return output;
}

const emptyZh = collectEmptyKeys(zhCN);
const emptyEn = collectEmptyKeys(enUS);

if (emptyZh.length > 0) {
  throw new Error(
    `i18n dictionary check failed: ${emptyZh.length} empty/non-string value(s) in zh-CN.js:\n` +
      emptyZh.map((key) => `  - ${key}`).join("\n"),
  );
}

if (emptyEn.length > 0) {
  throw new Error(
    `i18n dictionary check failed: ${emptyEn.length} empty/non-string value(s) in en-US.js:\n` +
      emptyEn.map((key) => `  - ${key}`).join("\n"),
  );
}

const moduleSource = await readFile(i18nModule, "utf8");
for (const exported of ["t", "tf", "setLocale", "getLocale", "registerLocale"]) {
  if (!new RegExp(`export\\s+function\\s+${exported}\\s*\\(`).test(moduleSource)) {
    throw new Error(`i18n module check failed: missing export "${exported}" in src/i18n.js`);
  }
}

console.log(
  `i18n check passed: ${zhKeys.size} keys, zh-CN and en-US in parity`,
);
