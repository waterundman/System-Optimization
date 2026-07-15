import { readdir, readFile } from "node:fs/promises";
import { extname, resolve } from "node:path";

const forbidden = [
  /from\s+["']react["']/,
  /from\s+["'][^"']*tauri[^"']*["']/i,
  /from\s+["'][^"']*sqlite[^"']*["']/i,
  /from\s+["'][^"']*obsidian[^"']*["']/i,
  /from\s+["'][^"']*openai[^"']*["']/i,
  /from\s+["'][^"']*ollama[^"']*["']/i,
  /node:(fs|path|crypto|http|https)/,
];

async function walk(dir) {
  const output = [];
  for (const entry of await readdir(dir, { withFileTypes: true })) {
    const path = resolve(dir, entry.name);
    if (entry.isDirectory()) output.push(...(await walk(path)));
    else if ([".ts", ".tsx"].includes(extname(entry.name))) output.push(path);
  }
  return output;
}

const roots = [
  "packages/kernel/src",
  "packages/editor-bridge/src",
  "packages/patch-engine/src",
  "packages/model-gateway/src",
  "packages/operation-runner/src",
];
const files = (await Promise.all(roots.map((root) => walk(resolve(root))))).flat();
const violations = [];
for (const file of files) {
  const source = await readFile(file, "utf8");
  for (const rule of forbidden) {
    if (rule.test(source)) violations.push(`${file}: ${rule}`);
  }
}

if (violations.length) {
  throw new Error(`Core architecture violations:\n${violations.join("\n")}`);
}

console.log(`Architecture check passed: ${files.length} core source files`);
