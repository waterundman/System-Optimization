import { cp, mkdir, readFile, readdir, rm, writeFile } from "node:fs/promises";
import { stripTypeScriptTypes } from "node:module";
import { dirname, extname, relative, resolve } from "node:path";
import { fileURLToPath } from "node:url";

const repository = resolve(dirname(fileURLToPath(import.meta.url)), "..");
const source = resolve(repository, "apps/optimizer-desktop/src");
const destination = resolve(repository, "apps/optimizer-desktop/dist");

if (dirname(destination) !== resolve(repository, "apps/optimizer-desktop")) {
  throw new Error(`Refusing to replace unexpected desktop output: ${destination}`);
}

for (const required of [
  "index.html",
  "main.js",
  "frontend-state.js",
  "operation-client.js",
  "styles.css",
]) {
  const content = await readFile(resolve(source, required), "utf8");
  if (!content.trim()) throw new Error(`Desktop source is empty: ${required}`);
  if (/https?:\/\//u.test(content)) {
    throw new Error(`Desktop source must not load remote assets: ${required}`);
  }
}

await rm(destination, { recursive: true, force: true });
await mkdir(destination, { recursive: true });
await cp(source, destination, { recursive: true });
for (const packageName of [
  "protocol",
  "kernel",
  "editor-bridge",
  "patch-engine",
  "model-gateway",
  "operation-runner",
]) {
  const packageSource = resolve(repository, "packages", packageName, "src");
  const packageDestination = resolve(destination, "runtime", "packages", packageName, "src");
  for (const input of await sourceFiles(packageSource)) {
    const output = resolve(
      packageDestination,
      relative(packageSource, input).replace(/\.ts$/u, ".js"),
    );
    const code = await readFile(input, "utf8");
    const transformed = stripTypeScriptTypes(code, {
      mode: "transform",
      sourceMap: false,
    }).replace(/(from\s+["'][^"']+)\.ts(["'])/gu, "$1.js$2");
    await mkdir(dirname(output), { recursive: true });
    await writeFile(output, transformed, "utf8");
  }
}
console.log(`Desktop frontend built: ${destination}`);

async function sourceFiles(directory) {
  const entries = await readdir(directory, { withFileTypes: true });
  const files = [];
  for (const entry of entries) {
    const path = resolve(directory, entry.name);
    if (entry.isDirectory()) files.push(...await sourceFiles(path));
    else if (entry.isFile() && extname(entry.name) === ".ts") files.push(path);
  }
  return files.sort();
}
