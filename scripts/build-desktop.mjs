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

// Every frontend source file must be non-empty and must not load remote
// assets: the desktop runs under the CSP-controlled asset protocol and any
// https:// in a bundled script would be a network-exfiltration vector.
for (const file of await sourceFiles(source, [".js", ".html", ".css"])) {
  const content = await readFile(file, "utf8");
  if (!content.trim()) throw new Error(`Desktop source is empty: ${relative(source, file)}`);
  if (/https?:\/\//u.test(content)) {
    throw new Error(`Desktop source must not load remote assets: ${relative(source, file)}`);
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
    let transformed;
    try {
      transformed = stripTypeScriptTypes(code, {
        mode: "transform",
        sourceMap: false,
      })
        .replace(/(from\s+["'][^"']+)\.ts(["'])/gu, "$1.js$2")
        // Rewrite @optimizer/* workspace bare imports to the flat sibling
        // layout of dist/runtime/packages/<pkg>/src so the desktop bundle
        // resolves one module graph instead of mixing bundled JS with the
        // root workspace source files.
        .replace(/(from\s+["'])@optimizer\/([^"']+)(["'])/gu, "$1../../$2/src/index.js$3");
    } catch (error) {
      throw new Error(
        `Failed to strip TypeScript types from ${relative(repository, input)}: ${
          error instanceof Error ? error.message : String(error)
        }`,
      );
    }
    await mkdir(dirname(output), { recursive: true });
    await writeFile(output, transformed, "utf8");
  }
}
console.log(`Desktop frontend built: ${destination}`);

async function sourceFiles(directory, extensions = [".ts"]) {
  const entries = await readdir(directory, { withFileTypes: true });
  const files = [];
  for (const entry of entries) {
    const path = resolve(directory, entry.name);
    if (entry.isDirectory()) files.push(...await sourceFiles(path, extensions));
    else if (entry.isFile() && extensions.includes(extname(entry.name))) files.push(path);
  }
  return files.sort();
}
