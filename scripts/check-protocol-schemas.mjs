import { readdir, readFile } from "node:fs/promises";
import { resolve } from "node:path";

const root = resolve("packages/protocol/schemas");
const files = (await readdir(root)).filter((name) => name.endsWith(".schema.json"));

if (files.length < 4) {
  throw new Error(`Expected at least four protocol schemas, found ${files.length}`);
}

for (const file of files) {
  const schema = JSON.parse(await readFile(resolve(root, file), "utf8"));
  if (schema.$schema !== "https://json-schema.org/draft/2020-12/schema") {
    throw new Error(`${file}: unsupported or missing $schema`);
  }
  if (!schema.$id || !schema.title || schema.type !== "object") {
    throw new Error(`${file}: $id, title and object type are required`);
  }
}

console.log(`Schema check passed: ${files.length} files`);

