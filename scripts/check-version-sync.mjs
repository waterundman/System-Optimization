import { readFile } from "node:fs/promises";
import { resolve, dirname } from "node:path";
import { fileURLToPath } from "node:url";

// v0.8.0 Stage 4: Version consistency guard for release engineering.
// Verifies that the desktop app version stays in sync across the Tauri
// manifest, the workspace package.json, and the README, and that the
// store schema version declared in migration.rs matches the number of
// MIGRATION_N constants actually defined there. A drift here would
// silently ship a release with mismatched metadata or a half-applied
// schema migration, so this runs as part of `npm run check`.

const root = resolve(dirname(fileURLToPath(import.meta.url)), "..");

const tauriConfPath = resolve(root, "apps/optimizer-desktop/src-tauri/tauri.conf.json");
const packageJsonPath = resolve(root, "package.json");
const readmePath = resolve(root, "README.md");
const migrationPath = resolve(root, "crates/optimizer-store/src/migration.rs");

const tauriConf = JSON.parse(await readFile(tauriConfPath, "utf8"));
const packageJson = JSON.parse(await readFile(packageJsonPath, "utf8"));
const readme = await readFile(readmePath, "utf8");
const migrationSource = await readFile(migrationPath, "utf8");

const tauriVersion = tauriConf.version;
const packageVersion = packageJson.version;

if (!tauriVersion) {
  throw new Error("version-sync failed: tauri.conf.json is missing the version field");
}
if (!packageVersion) {
  throw new Error("version-sync failed: package.json is missing the version field");
}
if (tauriVersion !== packageVersion) {
  throw new Error(
    `version-sync failed: tauri.conf.json version "${tauriVersion}" does not match package.json version "${packageVersion}"`,
  );
}

if (!readme.includes(tauriVersion)) {
  throw new Error(
    `version-sync failed: README.md does not mention the current version "${tauriVersion}"`,
  );
}

const schemaVersionMatch = migrationSource.match(
  /CURRENT_SCHEMA_VERSION\s*:\s*i64\s*=\s*(\d+)/,
);
if (!schemaVersionMatch) {
  throw new Error(
    "version-sync failed: could not find CURRENT_SCHEMA_VERSION in migration.rs",
  );
}
const declaredSchemaVersion = Number(schemaVersionMatch[1]);

const migrationDeclarations = [
  ...migrationSource.matchAll(/(?:pub\s+)?const\s+MIGRATION_(\d+)\s*:/g),
];
const migrationCount = migrationDeclarations.length;
const migrationNumbers = migrationDeclarations
  .map((match) => Number(match[1]))
  .sort((a, b) => a - b);

if (migrationCount === 0) {
  throw new Error(
    "version-sync failed: no MIGRATION_N constants found in migration.rs",
  );
}
if (migrationCount !== declaredSchemaVersion) {
  throw new Error(
    `version-sync failed: CURRENT_SCHEMA_VERSION is ${declaredSchemaVersion} but migration.rs defines ${migrationCount} MIGRATION_N constant(s)`,
  );
}

const expectedNumbers = Array.from({ length: declaredSchemaVersion }, (_, i) => i + 1);
const missingMigrations = expectedNumbers.filter(
  (n) => !migrationNumbers.includes(n),
);
if (missingMigrations.length > 0) {
  throw new Error(
    `version-sync failed: migration.rs is missing MIGRATION_N constant(s) for version(s): ${missingMigrations.join(", ")}`,
  );
}

console.log(
  `version-sync passed: app v${tauriVersion}, schema v${declaredSchemaVersion} (${migrationCount} migrations)`,
);
