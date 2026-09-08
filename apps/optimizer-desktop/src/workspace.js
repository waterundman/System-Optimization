import { t } from "./i18n.js";
import {
  element,
  button,
  svgIcon,
  iconOnlyButton,
  iconTextButton,
} from "./dom.js";
import { invokeHost } from "./client.js";
import {
  normalizeHostError,
  selectInitialDocument,
  blocksForDocument,
  isEditableBlock,
  countVisibleCharacters,
} from "./frontend-state.js";
import { PROVIDER_PRESETS } from "./operation-client.js";
import { renderWelcome } from "./welcome.js";
import {
  toggleCompare,
  toggleInsights,
  toggleCandidates,
  toggleVersions,
  toggleTimeline,
  toggleStyles,
  toggleProviders,
  toggleBackupWizard,
  providerDrawer,
  styleDrawer,
  candidateDrawer,
  versionDrawer,
  timelineDrawer,
  insightsDrawer,
  compareDrawer,
  backupRestoreWizard,
  pinCurrentStyleSample,
} from "./drawers.js";
import { openCommandPalette, commandPaletteModal, aiContextMenuView, openAiContextMenu } from "./palette.js";
import { aiReviewView } from "./review.js";
import {
  state,
  app,
  setNotice,
  noticeView,
  loadWorkspace,
  summaryStatusLabel,
  createCheckpoint,
  closeProject,
  contextPreviewModal,
  flushAll,
  queueSave,
  scheduleSummaryRefresh,
  retryConflictDraft,
  cancelAiOperation,
  runAiOperation,
  AI_OPERATION_COMMANDS,
} from "./main.js";

// Tear down virtualized list controllers (compare/timeline drawers) before
// replacing the app DOM. Each controller holds a ResizeObserver per rendered
// row plus a pending requestAnimationFrame; without this the observers and
// RAF handles would accumulate on every panel toggle / re-render.
function disposeVirtualControllers() {
  if (!app) return;
  const pending = [app];
  while (pending.length > 0) {
    const node = pending.pop();
    const controller = node?.__virtualController;
    if (controller && typeof controller.destroy === "function") {
      controller.destroy();
      node.__virtualController = null;
    }
    if (node?.children) {
      for (const child of node.children) pending.push(child);
    }
  }
}

function renderWorkspace() {
  disposeVirtualControllers();
  const project = state.session?.project;
  if (!project || !state.workspace) return renderWelcome();
  const activeCandidates = state.reviewCandidates.filter(
    (candidate) => candidate.status === "review" || candidate.status === "ready" || candidate.status === "conflicted",
  ).length;
  const topbar = element("header", { className: "topbar" }, [
    element("div", { className: "topbar-brand" }, [
      element("div", { className: "brand-mark", text: t("common.brandMark") }),
      element("div", {}, [
        element("strong", { text: project.title }),
      ]),
    ]),
    element("div", { className: "topbar-actions" }, [
      element("span", { className: "save-status", text: state.saveStatus, attrs: { id: "save-status" } }),
      element("span", {
        className: "summary-status",
        text: summaryStatusLabel(),
        title: t("toolbar.summaryInvalidations"),
        attrs: { id: "summary-status" },
      }),
      iconTextButton("download", t("toolbar.importMd"), "ghost-button", importMarkdown),
      iconTextButton("upload", t("toolbar.exportMd"), "ghost-button", exportMarkdown),
      iconTextButton("upload", t("toolbar.exportJson"), "ghost-button", exportJson),
      iconTextButton("flag", t("toolbar.checkpoint"), "ghost-button", createCheckpoint),
      iconOnlyButton("compare", `ghost-button${state.compareOpen ? " active" : ""}`, toggleCompare, {
        title: state.compareOpen ? t("toolbar.compareCollapse") : t("toolbar.compare"),
      }),
      iconOnlyButton("chart", `ghost-button${state.insightsOpen ? " active" : ""}`, toggleInsights, {
        title: state.insightsOpen ? t("toolbar.insightsCollapse") : t("toolbar.insights"),
      }),
      iconOnlyButton("layers", `ghost-button${state.candidatesOpen ? " active" : ""}`, toggleCandidates, {
        title: state.candidatesOpen
          ? t("toolbar.candidatesCollapse")
          : t("toolbar.candidateCenter", { count: activeCandidates }),
        attrs: {
          "aria-label": state.candidatesOpen
            ? t("toolbar.candidatesCollapse")
            : t("toolbar.candidateCenter", { count: activeCandidates }),
        },
      }),
      iconOnlyButton("history", `ghost-button${state.versionsOpen ? " active" : ""}`, toggleVersions, {
        title: state.versionsOpen ? t("toolbar.versionsCollapse") : t("toolbar.versions"),
      }),
      iconOnlyButton("timeline", `ghost-button${state.timelineOpen ? " active" : ""}`, toggleTimeline, {
        title: state.timelineOpen ? t("toolbar.timelineCollapse") : t("toolbar.timeline"),
      }),
      iconOnlyButton("book", `ghost-button${state.stylesOpen ? " active" : ""}`, toggleStyles, {
        title: state.stylesOpen ? t("toolbar.stylesCollapse") : t("toolbar.styles"),
      }),
      iconOnlyButton("settings", `ghost-button${state.providersOpen ? " active" : ""}`, toggleProviders, {
        title: state.providersOpen ? t("toolbar.providersCollapse") : t("toolbar.providers"),
      }),
      iconOnlyButton("backup", `ghost-button${state.backupWizardOpen ? " active" : ""}`, toggleBackupWizard, {
        title: t("wizard.title"),
      }),
      button(t("toolbar.closeProject"), "quiet-button", closeProject),
    ]),
  ]);
  const sidebar = documentSidebar();
  const editor = editorPane();
  const drawer = state.providersOpen
    ? providerDrawer()
    : state.stylesOpen
      ? styleDrawer()
      : state.candidatesOpen
        ? candidateDrawer()
        : state.versionsOpen
          ? versionDrawer()
          : state.timelineOpen
            ? timelineDrawer()
            : state.insightsOpen
              ? insightsDrawer()
              : state.compareOpen
                ? compareDrawer()
                : state.backupWizardOpen
                  ? backupRestoreWizard()
                  : null;
  const body = element("div", { className: `workspace-body${drawer ? " with-versions" : ""}` }, [
    sidebar,
    editor,
    drawer,
  ]);
  app.replaceChildren(element("div", { className: "app-shell" }, [
    topbar,
    noticeView(),
    body,
    contextPreviewModal(),
    commandPaletteModal(),
    aiContextMenuView(),
  ]));
}

function documentTreeEntries(documents) {
  const byId = new Map(documents.map((document) => [document.id, document]));
  const children = new Map();
  for (const document of documents) {
    const parentId = document.parentId && byId.has(document.parentId) ? document.parentId : null;
    const siblings = children.get(parentId) ?? [];
    siblings.push(document);
    children.set(parentId, siblings);
  }
  for (const siblings of children.values()) {
    siblings.sort((left, right) => left.orderKey.localeCompare(right.orderKey) || left.id.localeCompare(right.id));
  }
  const entries = [];
  const visited = new Set();
  const visit = (document, depth, siblingIndex, siblingCount) => {
    if (visited.has(document.id)) return;
    visited.add(document.id);
    entries.push({ document, depth, siblingIndex, siblingCount });
    const descendants = children.get(document.id) ?? [];
    descendants.forEach((child, index) => visit(child, depth + 1, index, descendants.length));
  };
  const roots = children.get(null) ?? [];
  roots.forEach((document, index) => visit(document, 0, index, roots.length));
  for (const document of documents) {
    if (!visited.has(document.id)) visit(document, 0, entries.length, documents.length);
  }
  return entries;
}

// O(n) descendant counts: build a single child index and sum in post-order
// so deep document trees never trigger a full documents.filter() per node
// (which made this quadratic for tall hierarchies).
function documentDescendantCounts(documents) {
  const byId = new Map(documents.map((document) => [document.id, document]));
  const children = new Map();
  for (const document of documents) {
    const parentId = document.parentId && byId.has(document.parentId) ? document.parentId : null;
    const siblings = children.get(parentId) ?? [];
    siblings.push(document);
    children.set(parentId, siblings);
  }
  const counts = new Map();
  const visit = (id) => {
    const cached = counts.get(id);
    if (cached !== undefined) return cached;
    let total = 0;
    for (const child of children.get(id) ?? []) {
      total += 1 + visit(child.id);
    }
    counts.set(id, total);
    return total;
  };
  for (const document of documents) visit(document.id);
  return counts;
}

function documentSidebar() {
  const documents = state.workspace.documents;
  const entries = documentTreeEntries(documents);
  const list = element("nav", { className: "document-list", attrs: { "aria-label": t("doc.ariaLabel") } });
  for (const entry of entries) {
    const { document, depth, siblingIndex, siblingCount } = entry;
    const select = button("", `document-button${document.id === state.selectedDocumentId ? " active" : ""}`, () => {
      state.selectedDocumentId = document.id;
      state.conflictDraft = null;
      renderWorkspace();
    });
    select.append(
      element("span", {
        className: "document-indent",
        text: "",
        attrs: { "aria-hidden": "true" },
      }),
      element("span", { className: "document-icon" }, [svgIcon(document.kind === "chapter" ? "chapter" : "text", 14)]),
      element("span", { className: "document-name", text: document.title }),
    );
    const actions = element("span", { className: "document-actions" }, [
      iconOnlyButton("up", "document-action", () => reorderDocument(document, "up"), {
        title: t("doc.moveUpTitle", { title: document.title }),
        disabled: siblingIndex === 0,
        attrs: { "aria-label": t("doc.moveUpLabel", { title: document.title }) },
      }),
      iconOnlyButton("down", "document-action", () => reorderDocument(document, "down"), {
        title: t("doc.moveDownTitle", { title: document.title }),
        disabled: siblingIndex === siblingCount - 1,
        attrs: { "aria-label": t("doc.moveDownLabel", { title: document.title }) },
      }),
      iconOnlyButton("edit", "document-action", () => renameDocument(document), {
        title: t("doc.renameTitle", { title: document.title }),
        attrs: { "aria-label": t("doc.renameLabel", { title: document.title }) },
      }),
      iconOnlyButton("close", "document-action document-action-danger", () => archiveDocument(document), {
        title: documents.length === 1 ? t("doc.archiveOnlyDoc") : t("doc.archiveTitle", { title: document.title }),
        disabled: documents.length === 1,
        attrs: { "aria-label": t("doc.archiveLabel", { title: document.title }) },
      }),
    ]);
    list.append(element("div", { className: "document-row", attrs: { "data-depth": String(depth) } }, [select, actions]));
    if (document.id === state.selectedDocumentId) {
      list.append(element("div", { className: "document-structure-actions" }, [
        iconOnlyButton("indent", "document-structure-action", () => changeDocumentDepth(document, "indent"), {
          disabled: siblingIndex === 0,
          attrs: { "aria-label": t("doc.indentLabel", { title: document.title }) },
        }),
        iconOnlyButton("outdent", "document-structure-action", () => changeDocumentDepth(document, "outdent"), {
          disabled: !document.parentId,
          attrs: { "aria-label": t("doc.outdentLabel", { title: document.title }) },
        }),
        iconOnlyButton("plus", "document-structure-action", () => createDocument(document.id), {
          attrs: { "aria-label": t("doc.addChildLabel", { title: document.title }) },
        }),
      ]));
    }
  }
  if (state.archivedDocuments.length) {
    list.append(element("div", { className: "archived-heading", text: t("doc.archivedHeading", { count: state.archivedDocuments.length }) }));
    for (const document of state.archivedDocuments) {
      list.append(element("div", { className: "archived-document-row" }, [
        element("span", { text: document.title, title: document.title }),
        iconOnlyButton("restore", "document-restore", () => restoreArchivedDocument(document), {
          attrs: { "aria-label": t("doc.restoreLabel", { title: document.title }) },
        }),
      ]));
    }
  }
  return element("aside", { className: "sidebar" }, [
    element("div", { className: "sidebar-heading" }, [
      element("span", { text: t("doc.listLabel") }),
      element("div", { className: "sidebar-heading-actions" }, [
        element("span", { className: "count-pill", text: String(documents.length) }),
        iconOnlyButton("plus", "icon-button document-add", () => createDocument(null), {
          title: t("doc.newTopLevel"),
          attrs: { "aria-label": t("doc.newTopLevel") },
        }),
      ]),
    ]),
    list,
    element("div", { className: "sidebar-stats" }, [
      element("span", { text: t("doc.validChars") }),
      element("strong", { text: countVisibleCharacters(state.workspace).toLocaleString("zh-CN") }),
    ]),
  ]);
}

async function commitDocumentMutation(command, input, preferredDocumentId, successMessage) {
  try {
    await flushAll();
    const response = await invokeHost(command, { input: { schemaVersion: 1, ...input } });
    state.workspace = response.workspace;
    [state.archivedDocuments, state.summaryInvalidations] = await Promise.all([
      invokeHost("list_archived_documents"),
      invokeHost("list_summary_invalidations"),
    ]);
    state.session = {
      ...state.session,
      project: {
        ...state.session.project,
        headCommitId: response.headCommitId,
        revision: response.projectRevision,
      },
    };
    state.selectedDocumentId = selectInitialDocument(response.workspace, preferredDocumentId);
    state.versionHistory = null;
    state.conflictDraft = null;
    state.activeBlockId = null;
    state.aiSelection = null;
    state.aiReview = null;
    state.aiRetry = null;
    setNotice("success", successMessage);
    renderWorkspace();
    scheduleSummaryRefresh();
  } catch (error) {
    setNotice("error", normalizeHostError(error).message);
    await loadWorkspace().catch(() => renderWorkspace());
  }
}

async function renameDocument(document) {
  const title = window.prompt(t("doc.renamePrompt"), document.title);
  if (title === null || title.trim() === document.title) return;
  await commitDocumentMutation(
    "rename_document",
    { documentId: document.id, expectedRevision: document.revision, title },
    document.id,
    t("doc.renamed", { title: title.trim() }),
  );
}

async function reorderDocument(document, direction) {
  await commitDocumentMutation(
    "reorder_document",
    { documentId: document.id, expectedRevision: document.revision, direction },
    document.id,
    t("doc.moved", { direction: direction === "up" ? t("doc.directionUp") : t("doc.directionDown"), title: document.title }),
  );
}

async function changeDocumentDepth(document, direction) {
  await commitDocumentMutation(
    "change_document_depth",
    { documentId: document.id, expectedRevision: document.revision, direction },
    document.id,
    direction === "indent"
      ? t("doc.indented", { title: document.title })
      : t("doc.outdented", { title: document.title }),
  );
}

async function archiveDocument(document) {
  const descendants = documentDescendantCounts(state.workspace.documents).get(document.id) ?? 0;
  const cascade = descendants ? t("doc.archiveCascade", { count: descendants }) : "";
  if (!window.confirm(t("doc.archiveConfirm", { title: document.title, cascade }))) return;
  await commitDocumentMutation(
    "set_document_archived",
    { documentId: document.id, expectedRevision: document.revision, archived: true },
    state.selectedDocumentId === document.id ? null : state.selectedDocumentId,
    t("doc.archived", { title: document.title }),
  );
}

async function restoreArchivedDocument(document) {
  await commitDocumentMutation(
    "set_document_archived",
    { documentId: document.id, expectedRevision: document.revision, archived: false },
    document.id,
    t("doc.restored", { title: document.title }),
  );
}

async function createDocument(parentDocumentId = null) {
  const parent = state.workspace.documents.find((document) => document.id === parentDocumentId);
  const title = window.prompt(parent ? t("doc.newChildPrompt", { title: parent.title }) : t("doc.newTitlePrompt"));
  if (title === null) return;
  try {
    await flushAll();
    const created = await invokeHost("create_document", {
      input: { schemaVersion: 1, title, initialText: "", parentDocumentId },
    });
    state.summaryInvalidations = await invokeHost("list_summary_invalidations");
    applyCreatedDocument(created, t("doc.created", { title: created.document.title }));
    scheduleSummaryRefresh();
  } catch (error) {
    setNotice("error", normalizeHostError(error).message);
    await loadWorkspace().catch(() => renderWorkspace());
  }
}

function applyCreatedDocument(created, message) {
  state.workspace = {
    ...state.workspace,
    revision: created.projectRevision,
    headCommitId: created.commitId,
    documents: [...state.workspace.documents, created.document],
    blocks: [...state.workspace.blocks, created.block],
  };
  if (state.session?.project) {
    state.session.project.revision = created.projectRevision;
    state.session.project.headCommitId = created.commitId;
  }
  state.selectedDocumentId = created.document.id;
  state.activeBlockId = created.block.id;
  state.aiSelection = null;
  state.versionHistory = null;
  setNotice("success", message);
  renderWorkspace();
}

function importMarkdown() {
  const picker = element("input", {
    type: "file",
    attrs: { accept: ".md,.markdown,text/markdown,text/plain", "aria-label": t("import.ariaLabel") },
  });
  picker.hidden = true;
  picker.addEventListener("change", async () => {
    const file = picker.files?.[0];
    picker.remove();
    if (!file) return;
    try {
      if (file.size > 2 * 1024 * 1024) {
        throw { code: "IMPORT_TOO_LARGE", message: t("error.importTooLarge") };
      }
      await flushAll();
      const text = (await file.text()).replace(/^\uFEFF/, "");
      const title = file.name.replace(/\.(?:md|markdown|txt)$/i, "") || t("doc.importedName");
      const created = await invokeHost("create_document", {
        input: { schemaVersion: 1, title, initialText: text },
      });
      state.summaryInvalidations = await invokeHost("list_summary_invalidations");
      applyCreatedDocument(created, t("doc.imported", { name: file.name }));
      scheduleSummaryRefresh();
    } catch (error) {
      setNotice("error", normalizeHostError(error).message);
      renderWorkspace();
    }
  }, { once: true });
  document.body.append(picker);
  picker.click();
}

async function exportMarkdown() {
  try {
    await flushAll();
    const exported = await invokeHost("export_markdown");
    setNotice("success", t("export.markdown", { count: exported.documents, path: exported.path }));
    renderWorkspace();
  } catch (error) {
    setNotice("error", normalizeHostError(error).message);
    renderWorkspace();
  }
}

async function exportJson() {
  try {
    await flushAll();
    const exported = await invokeHost("export_json");
    setNotice("success", t("export.json", { path: exported.path }));
    renderWorkspace();
  } catch (error) {
    setNotice("error", normalizeHostError(error).message);
    renderWorkspace();
  }
}

function editorPane() {
  const document = state.workspace.documents.find((item) => item.id === state.selectedDocumentId);
  if (!document) {
    return element("section", { className: "editor-empty", text: t("editor.empty") });
  }
  const toolbar = element("div", { className: "editor-toolbar" }, [
    element("span", { className: "toolbar-label", text: t("ai.toolbar.label") }),
    ...AI_OPERATION_COMMANDS.map((command) =>
      iconTextButton(command.icon, command.label, "tool-button", () => runAiOperation(command.type), {
        disabled: Boolean(state.aiRunning || state.aiReview?.kind === "patch_proposal"),
        title: `${command.description} · ${command.shortcut}`,
        attrs: { "aria-keyshortcuts": command.shortcut },
      }),
    ),
    button(t("ai.toolbar.pinStyle"), "tool-button style-pin-button", pinCurrentStyleSample, {
      disabled: Boolean(state.aiRunning || state.aiReview?.kind === "patch_proposal"),
      title: t("ai.toolbar.pinStyleTitle"),
    }),
    button(t("ai.toolbar.command"), "tool-button command-palette-trigger", openCommandPalette, {
      title: t("ai.toolbar.commandTitle"),
      attrs: { "aria-keyshortcuts": "Control+Shift+P Meta+Shift+P" },
    }),
    state.aiRunning
      ? button(t("common.cancel"), "danger-button", cancelAiOperation)
      : null,
    element("span", {
      className: "toolbar-note",
      text: state.aiRunning?.label
        ?? `${PROVIDER_PRESETS[state.selectedProviderId].label} · ${state.providerSettings[state.selectedProviderId].defaultModel}`,
      attrs: { id: "ai-status" },
    }),
  ]);
  const paper = element("article", { className: "editor-paper" }, [
    element("div", { className: "document-kicker", text: document.kind.toUpperCase() }),
    element("h1", { className: "document-title", text: document.title }),
    conflictView(),
    aiReviewView(),
  ]);
  for (const block of blocksForDocument(state.workspace, document.id)) paper.append(blockEditor(block));
  return element("section", { className: "editor-column" }, [toolbar, element("div", { className: "editor-scroll" }, [paper])]);
}

function conflictView() {
  const conflict = state.conflictDraft;
  if (!conflict || !state.workspace.blocks.some((block) => block.id === conflict.blockId)) return null;
  return element("div", { className: "conflict-card" }, [
    element("strong", { text: t("ai.conflict.title") }),
    element("p", { text: t("ai.conflict.description") }),
    element("div", { className: "conflict-actions" }, [
      button(t("ai.conflict.overwrite"), "danger-button", retryConflictDraft),
      button(t("ai.conflict.discard"), "quiet-button", () => {
        state.conflictDraft = null;
        renderWorkspace();
      }),
    ]),
  ]);
}

function blockEditor(block) {
  const underReview = state.aiReview?.kind === "patch_proposal"
    && state.aiReview.result.proposal.target.blockId === block.id;
  const editable = isEditableBlock(block) && !underReview;
  const wrapper = element("section", { className: `editor-block block-${block.kind}${editable ? "" : " locked"}` });
  const meta = element("div", { className: "block-meta" }, [
    element("span", { text: block.kind }),
  ]);
  const content = element("div", {
    className: "block-content",
    text: block.plainText,
    attrs: {
      "data-block-id": block.id,
      "data-placeholder": editable ? t("editor.placeholder.editable") : t("editor.placeholder.readonly"),
      role: "textbox",
      "aria-multiline": "true",
      spellcheck: "true",
    },
  });
  content.contentEditable = editable ? "plaintext-only" : "false";
  if (editable) {
    const rememberTarget = () => captureEditorTarget(block.id, content);
    content.addEventListener("focus", rememberTarget);
    content.addEventListener("click", rememberTarget);
    content.addEventListener("keyup", rememberTarget);
    content.addEventListener("mouseup", rememberTarget);
    content.addEventListener("input", () => {
      rememberTarget();
      queueSave(block.id, content.textContent ?? "");
    });
    content.addEventListener("contextmenu", (event) => {
      if (event.shiftKey) return;
      event.preventDefault();
      rememberTarget();
      openAiContextMenu(event.clientX, event.clientY, block.id);
    });
    content.addEventListener("keydown", (event) => {
      if ((event.ctrlKey || event.metaKey) && event.key.toLowerCase() === "s") {
        event.preventDefault();
        queueSave(block.id, content.textContent ?? "", true);
      }
    });
  }
  wrapper.append(meta, content);
  return wrapper;
}

function captureEditorTarget(blockId, content) {
  state.activeBlockId = blockId;
  const selection = window.getSelection();
  if (!selection?.rangeCount) {
    state.aiSelection = null;
    return;
  }
  const range = selection.getRangeAt(0);
  if (!content.contains(range.startContainer) || !content.contains(range.endContainer)) {
    state.aiSelection = null;
    return;
  }
  const from = textOffset(content, range.startContainer, range.startOffset);
  const to = textOffset(content, range.endContainer, range.endOffset);
  state.aiSelection = { blockId, from: Math.min(from, to), to: Math.max(from, to) };
}

function textOffset(root, node, offset) {
  const range = document.createRange();
  range.selectNodeContents(root);
  range.setEnd(node, offset);
  return range.toString().length;
}

function operationCommandsDisabled() {
  return Boolean(state.aiRunning || state.aiReview?.kind === "patch_proposal");
}

export {
  disposeVirtualControllers,
  renderWorkspace,
  documentTreeEntries,
  documentDescendantCounts,
  documentSidebar,
  commitDocumentMutation,
  renameDocument,
  reorderDocument,
  changeDocumentDepth,
  archiveDocument,
  restoreArchivedDocument,
  createDocument,
  applyCreatedDocument,
  importMarkdown,
  exportMarkdown,
  exportJson,
  editorPane,
  conflictView,
  blockEditor,
  captureEditorTarget,
  textOffset,
  operationCommandsDisabled,
};
