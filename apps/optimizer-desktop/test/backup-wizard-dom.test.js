import assert from "node:assert/strict";
import test from "node:test";
import { parseHTML } from "linkedom";

// Set up minimal browser globals before importing main.js so its top-level
// `document.querySelector("#app")` and `window.addEventListener` calls do not
// throw. Each individual test creates its own linkedom document and passes it
// directly to the function under test so the rendered tree is isolated from
// this bootstrap document.
const bootstrapDom = parseHTML(
  `<!DOCTYPE html><html><body><div id="app"></div></body></html>`,
);
global.document = bootstrapDom.document;
global.window = bootstrapDom.window;
if (!global.localStorage) {
  global.localStorage = {
    getItem: () => null,
    setItem: () => {},
    removeItem: () => {},
    clear: () => {},
  };
}
if (!global.crypto) {
  global.crypto = { subtle: {} };
}
if (typeof global.requestAnimationFrame !== "function") {
  global.requestAnimationFrame = (cb) => { cb(Date.now()); return 1; };
}
if (typeof global.cancelAnimationFrame !== "function") {
  global.cancelAnimationFrame = () => {};
}
if (typeof global.ResizeObserver !== "function") {
  global.ResizeObserver = class {
    observe() {}
    unobserve() {}
    disconnect() {}
  };
}

const {
  backupRestoreWizard,
  state,
  toggleBackupWizard,
} = await import("../dist/main.js");

function createDocument() {
  const { document } = parseHTML(
    `<!DOCTYPE html><html><body></body></html>`,
  );
  return document;
}

function resetWizardState() {
  state.backupWizardOpen = false;
  state.backupWizardStep = 1;
  state.backupWizardMode = null;
  state.backupWizardBusy = false;
  state.backupWizardResult = null;
  state.backupWizardManifest = null;
  state.backupWizardWarnings = [];
  state.backupWizardForm = {
    includeEndpoints: true,
    includeRecent: true,
    outputPath: "",
    archivePath: "",
    targetDirectory: "",
    newProjectId: "",
    overwrite: false,
  };
}

// T10: backupRestoreWizard renders a 3-step wizard UI.
test("T10: backupRestoreWizard renders three-step wizard UI with linkedom", () => {
  resetWizardState();
  const doc = createDocument();

  // Step 1: operation selection
  state.backupWizardOpen = true;
  state.backupWizardStep = 1;
  const drawer = backupRestoreWizard(doc);

  // Verify drawer container
  assert.ok(drawer, "backupRestoreWizard must return a DOM element");
  assert.ok(
    drawer.classList.contains("backup-wizard-drawer"),
    "drawer must have backup-wizard-drawer class",
  );

  // Verify step indicators
  const steps = drawer.querySelectorAll(".wizard-step");
  assert.equal(steps.length, 3, "must render 3 step indicators");

  // Step 1 should be active, steps 2 and 3 should not
  assert.ok(steps[0].classList.contains("active"), "step 1 must be active");
  assert.ok(!steps[1].classList.contains("active"), "step 2 must not be active");
  assert.ok(!steps[2].classList.contains("active"), "step 3 must not be active");

  // Verify radio buttons for backup/restore selection
  const radios = drawer.querySelectorAll("input[type='radio'][name='wizard-mode']");
  assert.equal(radios.length, 2, "must have 2 radio buttons for backup/restore");
  const values = Array.from(radios).map((r) => r.value);
  assert.ok(values.includes("backup"), "must have a 'backup' radio option");
  assert.ok(values.includes("restore"), "must have a 'restore' radio option");

  // Verify action buttons
  const buttons = drawer.querySelectorAll("button");
  const buttonTexts = Array.from(buttons).map((b) => b.textContent);
  assert.ok(
    buttonTexts.some((text) => text.includes("Next") || text.includes("下一步")),
    "must have a Next button",
  );
});

// T10 continued: Step 2 backup mode renders output path and checkboxes.
test("T10: Step 2 backup mode renders output path and include checkboxes", () => {
  resetWizardState();
  const doc = createDocument();

  state.backupWizardOpen = true;
  state.backupWizardStep = 2;
  state.backupWizardMode = "backup";

  const drawer = backupRestoreWizard(doc);

  // Verify output path input exists
  const outputInput = drawer.querySelector("input[name='outputPath']");
  assert.ok(outputInput, "must have an outputPath input");

  // Verify includeEndpoints checkbox
  const endpointsCheckbox = drawer.querySelector("input[name='includeEndpoints']");
  assert.ok(endpointsCheckbox, "must have an includeEndpoints checkbox");
  assert.ok(endpointsCheckbox.checked, "includeEndpoints should default to checked");

  // Verify includeRecent checkbox
  const recentCheckbox = drawer.querySelector("input[name='includeRecent']");
  assert.ok(recentCheckbox, "must have an includeRecent checkbox");
  assert.ok(recentCheckbox.checked, "includeRecent should default to checked");
});

// T10 continued: Step 2 restore mode renders archive path and target directory.
test("T10: Step 2 restore mode renders archive path and target directory fields", () => {
  resetWizardState();
  const doc = createDocument();

  state.backupWizardOpen = true;
  state.backupWizardStep = 2;
  state.backupWizardMode = "restore";

  const drawer = backupRestoreWizard(doc);

  // Verify archive path input
  const archiveInput = drawer.querySelector("input[name='archivePath']");
  assert.ok(archiveInput, "must have an archivePath input");

  // Verify target directory input
  const targetInput = drawer.querySelector("input[name='targetDirectory']");
  assert.ok(targetInput, "must have a targetDirectory input");

  // Verify new project ID input
  const newIdInput = drawer.querySelector("input[name='newProjectId']");
  assert.ok(newIdInput, "must have a newProjectId input");

  // Verify overwrite checkbox
  const overwriteCheckbox = drawer.querySelector("input[name='overwrite']");
  assert.ok(overwriteCheckbox, "must have an overwrite checkbox");
  assert.ok(!overwriteCheckbox.checked, "overwrite should default to unchecked");
});

// T11: backupRestoreWizard Step3 preview shows manifest content.
test("T11: Step3 backup preview displays form values correctly", () => {
  resetWizardState();
  const doc = createDocument();

  state.backupWizardOpen = true;
  state.backupWizardStep = 3;
  state.backupWizardMode = "backup";
  state.backupWizardForm.outputPath = "W:\\backups\\novel.optimizer-backup";
  state.backupWizardForm.includeEndpoints = true;
  state.backupWizardForm.includeRecent = false;
  state.backupWizardWarnings = [];

  const drawer = backupRestoreWizard(doc);

  // Verify preview section exists
  const preview = drawer.querySelector(".wizard-preview");
  assert.ok(preview, "must have a wizard-preview section");

  // Verify preview rows
  const rows = drawer.querySelectorAll(".preview-row");
  assert.ok(rows.length >= 3, "must have at least 3 preview rows");

  // Verify output path appears in preview
  const previewText = drawer.textContent;
  assert.ok(
    previewText.includes("W:\\backups\\novel.optimizer-backup"),
    "preview must show the output path",
  );
});

// T11 continued: Result view shows manifest fields.
test("T11: Result view displays manifest content after backup execution", () => {
  resetWizardState();
  const doc = createDocument();

  state.backupWizardOpen = true;
  state.backupWizardStep = 3;
  state.backupWizardMode = "backup";
  state.backupWizardResult = {
    schemaVersion: 1,
    archivePath: "W:\\backups\\novel.optimizer-backup",
    manifest: {
      schemaVersion: 1,
      sourceProjectId: "project-abc-123",
      sourcePath: "W:\\writing\\novel.optimizer",
      generatedAt: "2026-08-03T12:00:00.000Z",
      includedItems: ["project.sqlite3", "manifest.json", "endpoints.json"],
      optimizerVersion: "0.8.0",
      schemaDbVersion: 11,
    },
    bytesWritten: 4096,
    itemCount: 5,
  };

  const drawer = backupRestoreWizard(doc);

  // Verify the manifest content is displayed
  const text = drawer.textContent;
  assert.ok(
    text.includes("project-abc-123"),
    "result view must display source project ID",
  );
  assert.ok(
    text.includes("W:\\writing\\novel.optimizer"),
    "result view must display source path",
  );
  assert.ok(
    text.includes("2026-08-03T12:00:00.000Z"),
    "result view must display generated-at timestamp",
  );
  assert.ok(
    text.includes("project.sqlite3"),
    "result view must display included items",
  );
  assert.ok(
    text.includes("0.8.0"),
    "result view must display optimizer version",
  );
});

// T12: backupRestoreWizard Step3 explicitly warns that Secret is not included.
test("T12: Step3 backup preview shows Secret-not-included warning", () => {
  resetWizardState();
  const doc = createDocument();

  state.backupWizardOpen = true;
  state.backupWizardStep = 3;
  state.backupWizardMode = "backup";
  state.backupWizardWarnings = [
    "Backup does not contain API Keys; credentials must be re-entered after restore.",
  ];

  const drawer = backupRestoreWizard(doc);

  // Verify warning element exists and contains the Secret warning text
  const warnings = drawer.querySelectorAll(".wizard-warning");
  assert.ok(warnings.length > 0, "must have at least one warning element");

  const warningText = Array.from(warnings)
    .map((w) => w.textContent)
    .join(" ");
  assert.ok(
    warningText.includes("API Key") || warningText.includes("Secret"),
    "warning must explicitly mention API Key or Secret",
  );
});

// T12 continued: Restore Step3 also shows Secret warning.
test("T12: Step3 restore preview shows Secret warning", () => {
  resetWizardState();
  const doc = createDocument();

  state.backupWizardOpen = true;
  state.backupWizardStep = 3;
  state.backupWizardMode = "restore";
  state.backupWizardForm.archivePath = "W:\\backups\\novel.optimizer-backup";
  state.backupWizardForm.targetDirectory = "W:\\writing";
  state.backupWizardWarnings = [
    "Backup does not contain API Keys; credentials must be re-entered after restore.",
    "A new project ID will be generated after restore to avoid conflicts with the original project.",
  ];

  const drawer = backupRestoreWizard(doc);

  const warnings = drawer.querySelectorAll(".wizard-warning");
  assert.ok(warnings.length >= 2, "restore preview must have at least 2 warnings");

  const warningText = Array.from(warnings)
    .map((w) => w.textContent)
    .join(" ");
  assert.ok(
    warningText.includes("API Key") || warningText.includes("Secret"),
    "restore warning must explicitly mention API Key or Secret",
  );
  assert.ok(
    warningText.includes("project ID") || warningText.includes("project_id"),
    "restore warning must mention new project ID",
  );
});
