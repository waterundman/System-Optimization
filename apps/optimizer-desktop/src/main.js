import {
  SAVE_DEBOUNCE_MS,
  applySaveResponse,
  blocksForDocument,
  buildSaveBlockRequest,
  countVisibleCharacters,
  isEditableBlock,
  normalizeHostError,
  selectInitialDocument,
} from "./frontend-state.js";
import {
  PROVIDER_PRESETS,
  compileDesktopReview,
  createDesktopReview,
  credentialReference,
  decideDesktopHunk,
  defaultProviderSettings,
  hydrateDesktopReviewCandidate,
  persistDesktopReviewDecision,
  planDesktopReviewDecisions,
  providerRequiresCredential,
  rejectDesktopReview,
  runDesktopOperation,
} from "./operation-client.js";

const app = document.querySelector("#app");
const PROVIDER_SETTINGS_KEY = "optimizer.provider-settings.v1";
const SUMMARY_REFRESH_DEBOUNCE_MS = 800;
const AI_OPERATION_COMMANDS = Object.freeze([
  { type: "continue_scene", label: "续写", description: "从当前光标继续创作", shortcut: "Alt+1", key: "1" },
  { type: "polish", label: "润色", description: "保持原意优化表达", shortcut: "Alt+2", key: "2" },
  { type: "compress", label: "压缩", description: "收紧当前选区或 Block", shortcut: "Alt+3", key: "3" },
  { type: "expand", label: "扩写", description: "补充细节与过渡", shortcut: "Alt+4", key: "4" },
  { type: "critique", label: "批评", description: "只返回问题与建议", shortcut: "Alt+5", key: "5" },
]);
const state = {
  session: null,
  workspace: null,
  selectedDocumentId: null,
  versionsOpen: false,
  candidatesOpen: false,
  providersOpen: false,
  stylesOpen: false,
  styleSamples: [],
  knowledgeItems: [],
  archivedDocuments: [],
  summaryInvalidations: [],
  summaryRefreshTimer: null,
  summaryRefreshBusy: false,
  summaryRefreshFailures: 0,
  recentProjects: [],
  recentProjectBusy: null,
  selectedProviderId: "deepseek",
  providerSettings: loadProviderSettings(),
  ollamaDiscovery: null,
  versionHistory: null,
  reviewCandidates: [],
  candidateBusy: null,
  saveTimers: new Map(),
  pendingText: new Map(),
  savePromises: new Map(),
  saveStatus: "已保存",
  notice: null,
  conflictDraft: null,
  activeBlockId: null,
  aiSelection: null,
  aiRunning: null,
  aiContextPreview: null,
  aiReview: null,
  aiContextMenu: null,
  commandPaletteOpen: false,
};

function element(tag, options = {}, children = []) {
  const item = document.createElement(tag);
  if (options.className) item.className = options.className;
  if (options.text !== undefined) item.textContent = options.text;
  if (options.type) item.type = options.type;
  if (options.name) item.name = options.name;
  if (options.value !== undefined) item.value = options.value;
  if (options.placeholder) item.placeholder = options.placeholder;
  if (options.title) item.title = options.title;
  if (options.disabled) item.disabled = true;
  if (options.attrs) {
    for (const [name, value] of Object.entries(options.attrs)) item.setAttribute(name, String(value));
  }
  for (const child of children.flat()) if (child) item.append(child);
  return item;
}

function button(label, className, onClick, options = {}) {
  const item = element("button", { className, text: label, type: "button", ...options });
  if (onClick) item.addEventListener("click", onClick);
  return item;
}

async function invokeHost(command, args = {}) {
  const invoke = window.__TAURI__?.core?.invoke;
  if (typeof invoke !== "function") {
    throw { code: "HOST_UNAVAILABLE", message: "请通过 Optimizer System 桌面程序打开此界面" };
  }
  return invoke(command, args);
}

function setNotice(kind, message) {
  state.notice = message ? { kind, message } : null;
}

function noticeView() {
  if (!state.notice) return null;
  return element("div", {
    className: `notice notice-${state.notice.kind}`,
    text: state.notice.message,
    attrs: { role: "status" },
  });
}

function loadProviderSettings() {
  const defaults = defaultProviderSettings();
  try {
    const saved = JSON.parse(localStorage.getItem(PROVIDER_SETTINGS_KEY) ?? "null");
    if (!saved || typeof saved !== "object" || Array.isArray(saved)) return defaults;
    for (const providerId of Object.keys(PROVIDER_PRESETS)) {
      const value = saved[providerId];
      if (!value || typeof value !== "object" || Array.isArray(value)) continue;
      defaults[providerId] = {
        ...defaults[providerId],
        enabled: value.enabled === true,
        defaultModel: typeof value.defaultModel === "string"
          ? value.defaultModel
          : defaults[providerId].defaultModel,
        qwenRegion: typeof value.qwenRegion === "string" ? value.qwenRegion : "china",
        qwenWorkspaceId: typeof value.qwenWorkspaceId === "string" ? value.qwenWorkspaceId : "",
      };
    }
  } catch {
    // Invalid machine-local preferences fall back to safe provider defaults.
  }
  return defaults;
}

function persistProviderSettings() {
  const safe = Object.fromEntries(Object.entries(state.providerSettings).map(([providerId, value]) => [
    providerId,
    {
      enabled: value.enabled === true,
      defaultModel: value.defaultModel,
      qwenRegion: value.qwenRegion,
      qwenWorkspaceId: value.qwenWorkspaceId,
    },
  ]));
  localStorage.setItem(PROVIDER_SETTINGS_KEY, JSON.stringify(safe));
}

async function refreshProviderSecretStatus() {
  const entries = await Promise.all(Object.keys(PROVIDER_PRESETS).map(async (providerId) => {
    if (!providerRequiresCredential(providerId)) return [providerId, false];
    try {
      const status = await invokeHost("has_provider_secret", {
        reference: credentialReference(providerId),
      });
      return [providerId, status.exists === true];
    } catch {
      return [providerId, false];
    }
  }));
  for (const [providerId, credentialExists] of entries) {
    state.providerSettings[providerId] = {
      ...state.providerSettings[providerId],
      credentialExists,
    };
  }
}

async function bootstrap() {
  try {
    state.session = await invokeHost("get_project_session");
    if (state.session.isOpen) await loadWorkspace();
    else {
      await loadRecentProjects();
      renderWelcome();
    }
  } catch (error) {
    setNotice("error", normalizeHostError(error).message);
    renderWelcome();
  }
}

function renderWelcome() {
  const brand = element("section", { className: "welcome-brand" }, [
    element("div", { className: "brand-mark brand-mark-large", text: "优" }),
    element("p", { className: "eyebrow", text: "OPTIMIZER KERNEL" }),
    element("h1", { text: "把注意力留给文字" }),
    element("p", {
      className: "welcome-copy",
      text: "本地优先、版本安全的 AI 协作写作空间。每一次保存都有基线，每一次恢复都留下历史。",
    }),
    element("div", { className: "trust-list" }, [
      element("span", { text: "本地项目包" }),
      element("span", { text: "可审计版本" }),
      element("span", { text: "凭据不进 WebView" }),
    ]),
  ]);

  const createForm = element("form", { className: "start-form" }, [
    element("h2", { text: "创建新作品" }),
    labeledInput("作品名称", "title", "例如：雾港来信", true),
    labeledInput("保存到", "parentDirectory", "绝对目录，例如 W:\\写作", true),
    labeledInput("项目包名称", "folderName", "例如：雾港来信.optimizer", true),
    labeledInput("语言", "language", "zh-CN", true, "zh-CN"),
    element("p", { className: "form-hint", text: "项目会创建为独立的 .optimizer 目录包，不覆盖已有目录。" }),
    element("button", { className: "primary-button", text: "创建并进入", type: "submit" }),
  ]);
  createForm.addEventListener("submit", createProject);

  const openForm = element("form", { className: "start-form start-form-secondary" }, [
    element("h2", { text: "打开已有项目" }),
    labeledInput("项目包路径", "projectDirectory", "例如 W:\\写作\\雾港来信.optimizer", true),
    element("p", { className: "form-hint", text: "打开前会校验 manifest、SQLite 完整性、项目与主分支绑定。" }),
    element("button", { className: "secondary-button", text: "打开项目", type: "submit" }),
  ]);
  openForm.addEventListener("submit", openProject);

  const panel = element("section", { className: "welcome-panel" }, [
    noticeView(),
    recentProjectsView(),
    createForm,
    openForm,
  ]);
  app.replaceChildren(element("div", { className: "welcome-layout" }, [brand, panel]));
}

async function loadRecentProjects() {
  try {
    state.recentProjects = await invokeHost("list_recent_projects");
  } catch (error) {
    state.recentProjects = [];
    setNotice("warning", normalizeHostError(error).message);
  }
}

function recentProjectsView() {
  if (!state.recentProjects.length) return null;
  return element("section", { className: "recent-projects", attrs: { "aria-labelledby": "recent-projects-title" } }, [
    element("div", { className: "recent-projects-heading" }, [
      element("h2", { text: "最近项目", attrs: { id: "recent-projects-title" } }),
      element("span", { text: `${state.recentProjects.length} 个` }),
    ]),
    ...state.recentProjects.map(recentProjectRow),
  ]);
}

function recentProjectRow(project) {
  const busy = state.recentProjectBusy === project.projectId;
  const open = button("", "recent-project-open", () => openRecentProject(project), {
    disabled: busy || !project.available,
    attrs: { "aria-label": `打开最近项目 ${project.title}` },
  });
  open.append(
    element("span", { className: "recent-project-mark", text: "优" }),
    element("span", { className: "recent-project-copy" }, [
      element("strong", { text: project.title }),
      element("span", { text: project.available ? project.directory : "项目路径不可用" }),
    ]),
    element("span", { className: "recent-project-time", text: formatRecentTime(project.lastOpenedAt) }),
  );
  return element("div", { className: `recent-project${project.available ? "" : " recent-project-missing"}` }, [
    open,
    button("移除", "recent-project-remove", () => removeRecentProject(project), {
      disabled: busy,
      attrs: { "aria-label": `移除最近项目 ${project.title}` },
    }),
  ]);
}

function formatRecentTime(value) {
  const date = new Date(value);
  if (Number.isNaN(date.getTime())) return "最近打开";
  return new Intl.DateTimeFormat("zh-CN", { month: "numeric", day: "numeric" }).format(date);
}

async function openRecentProject(project) {
  if (!project.available || state.recentProjectBusy) return;
  state.recentProjectBusy = project.projectId;
  renderWelcome();
  try {
    state.session = await invokeHost("open_recent_project", {
      input: { schemaVersion: 1, projectId: project.projectId },
    });
    state.recentProjectBusy = null;
    setNotice(null, null);
    await loadWorkspace();
  } catch (error) {
    state.recentProjectBusy = null;
    setNotice("error", normalizeHostError(error).message);
    await loadRecentProjects();
    renderWelcome();
  }
}

async function removeRecentProject(project) {
  if (state.recentProjectBusy) return;
  state.recentProjectBusy = project.projectId;
  renderWelcome();
  try {
    await invokeHost("remove_recent_project", {
      input: { schemaVersion: 1, projectId: project.projectId },
    });
    state.recentProjects = state.recentProjects.filter((item) => item.projectId !== project.projectId);
    setNotice("success", `已从最近项目中移除“${project.title}”，项目文件未被删除。`);
  } catch (error) {
    setNotice("error", normalizeHostError(error).message);
  } finally {
    state.recentProjectBusy = null;
    renderWelcome();
  }
}

function labeledInput(label, name, placeholder, required, value = "") {
  const input = element("input", { name, placeholder, value, attrs: { autocomplete: "off" } });
  input.required = required;
  return element("label", { className: "field" }, [element("span", { text: label }), input]);
}

function formValue(form, name) {
  return String(new FormData(form).get(name) ?? "").trim();
}

async function createProject(event) {
  event.preventDefault();
  const form = event.currentTarget;
  setFormBusy(form, true);
  try {
    state.session = await invokeHost("create_project", {
      input: {
        schemaVersion: 1,
        parentDirectory: formValue(form, "parentDirectory"),
        folderName: formValue(form, "folderName"),
        title: formValue(form, "title"),
        language: formValue(form, "language"),
      },
    });
    setNotice(null, null);
    await loadWorkspace();
  } catch (error) {
    setNotice("error", normalizeHostError(error).message);
    renderWelcome();
  } finally {
    setFormBusy(form, false);
  }
}

async function openProject(event) {
  event.preventDefault();
  const form = event.currentTarget;
  setFormBusy(form, true);
  try {
    state.session = await invokeHost("open_project", {
      input: { schemaVersion: 1, projectDirectory: formValue(form, "projectDirectory") },
    });
    setNotice(null, null);
    await loadWorkspace();
  } catch (error) {
    setNotice("error", normalizeHostError(error).message);
    renderWelcome();
  } finally {
    setFormBusy(form, false);
  }
}

function setFormBusy(form, busy) {
  for (const control of form.elements) control.disabled = busy;
}

async function loadWorkspace() {
  const preferred = state.selectedDocumentId;
  const [workspace, styleSamples, knowledgeItems, archivedDocuments, summaryInvalidations, reviewCandidates] = await Promise.all([
    invokeHost("get_project_workspace"),
    invokeHost("list_style_samples"),
    invokeHost("list_knowledge_items"),
    invokeHost("list_archived_documents"),
    invokeHost("list_summary_invalidations"),
    invokeHost("list_review_candidates"),
  ]);
  state.workspace = workspace;
  state.styleSamples = styleSamples;
  state.knowledgeItems = knowledgeItems;
  state.archivedDocuments = archivedDocuments;
  state.summaryInvalidations = summaryInvalidations;
  state.reviewCandidates = reviewCandidates;
  state.selectedDocumentId = selectInitialDocument(state.workspace, preferred);
  await refreshProviderSecretStatus();
  renderWorkspace();
  scheduleSummaryRefresh();
}

function renderWorkspace() {
  const project = state.session?.project;
  if (!project || !state.workspace) return renderWelcome();
  const activeCandidates = state.reviewCandidates.filter(
    (candidate) => candidate.status === "review" || candidate.status === "ready" || candidate.status === "conflicted",
  ).length;
  const topbar = element("header", { className: "topbar" }, [
    element("div", { className: "topbar-brand" }, [
      element("div", { className: "brand-mark", text: "优" }),
      element("div", {}, [
        element("strong", { text: project.title }),
        element("span", { text: `${project.language} · r${state.workspace.revision}` }),
      ]),
    ]),
    element("div", { className: "topbar-actions" }, [
      element("span", { className: "save-status", text: state.saveStatus, attrs: { id: "save-status" } }),
      element("span", {
        className: "summary-status",
        text: summaryStatusLabel(),
        title: "正文或结构变化后合并产生的分层摘要失效项",
        attrs: { id: "summary-status" },
      }),
      button("导入 MD", "ghost-button", importMarkdown),
      button("导出 MD", "ghost-button", exportMarkdown),
      button("建立检查点", "ghost-button", createCheckpoint),
      button(
        state.candidatesOpen ? "收起候选" : "候选 " + activeCandidates,
        "ghost-button",
        toggleCandidates,
        {
          attrs: {
            "aria-label": state.candidatesOpen
              ? "收起候选"
              : "候选中心，" + activeCandidates + " 个待处理",
          },
        },
      ),
      button(state.versionsOpen ? "收起版本" : "版本历史", "ghost-button", toggleVersions),
      button(state.stylesOpen ? "收起知识" : "知识 / 风格", "ghost-button", toggleStyles),
      button(state.providersOpen ? "收起模型" : "模型设置", "ghost-button", toggleProviders),
      button("关闭项目", "quiet-button", closeProject),
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

function documentDescendantCount(documentId, documents) {
  let frontier = [documentId];
  let count = 0;
  while (frontier.length) {
    const parentId = frontier.pop();
    const children = documents.filter((document) => document.parentId === parentId);
    count += children.length;
    frontier.push(...children.map((document) => document.id));
  }
  return count;
}

function documentSidebar() {
  const documents = state.workspace.documents;
  const entries = documentTreeEntries(documents);
  const list = element("nav", { className: "document-list", attrs: { "aria-label": "文档" } });
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
        text: depth ? "· ".repeat(depth) : "",
        attrs: { "aria-hidden": "true" },
      }),
      element("span", { className: "document-icon", text: document.kind === "chapter" ? "章" : "文" }),
      element("span", { className: "document-name", text: document.title }),
    );
    const actions = element("span", { className: "document-actions" }, [
      button("↑", "document-action", () => reorderDocument(document, "up"), {
        title: `上移“${document.title}”`,
        disabled: siblingIndex === 0,
        attrs: { "aria-label": `上移 ${document.title}` },
      }),
      button("↓", "document-action", () => reorderDocument(document, "down"), {
        title: `下移“${document.title}”`,
        disabled: siblingIndex === siblingCount - 1,
        attrs: { "aria-label": `下移 ${document.title}` },
      }),
      button("✎", "document-action", () => renameDocument(document), {
        title: `重命名“${document.title}”`,
        attrs: { "aria-label": `重命名 ${document.title}` },
      }),
      button("×", "document-action document-action-danger", () => archiveDocument(document), {
        title: documents.length === 1 ? "项目必须保留一个有效章节" : `归档“${document.title}”`,
        disabled: documents.length === 1,
        attrs: { "aria-label": `归档 ${document.title}` },
      }),
    ]);
    list.append(element("div", { className: "document-row" }, [select, actions]));
    if (document.id === state.selectedDocumentId) {
      list.append(element("div", { className: "document-structure-actions" }, [
        button("缩进", "document-structure-action", () => changeDocumentDepth(document, "indent"), {
          disabled: siblingIndex === 0,
          attrs: { "aria-label": `缩进 ${document.title}` },
        }),
        button("移出", "document-structure-action", () => changeDocumentDepth(document, "outdent"), {
          disabled: !document.parentId,
          attrs: { "aria-label": `移出 ${document.title}` },
        }),
        button("＋ 子章节", "document-structure-action", () => createDocument(document.id), {
          attrs: { "aria-label": `新建 ${document.title} 的子章节` },
        }),
      ]));
    }
  }
  if (state.archivedDocuments.length) {
    list.append(element("div", { className: "archived-heading", text: `已归档 · ${state.archivedDocuments.length}` }));
    for (const document of state.archivedDocuments) {
      list.append(element("div", { className: "archived-document-row" }, [
        element("span", { text: document.title, title: document.title }),
        button("恢复", "document-restore", () => restoreArchivedDocument(document), {
          attrs: { "aria-label": `恢复 ${document.title}` },
        }),
      ]));
    }
  }
  return element("aside", { className: "sidebar" }, [
    element("div", { className: "sidebar-heading" }, [
      element("span", { text: "文档" }),
      element("div", { className: "sidebar-heading-actions" }, [
        element("span", { className: "count-pill", text: String(documents.length) }),
        button("＋", "icon-button document-add", () => createDocument(null), {
          title: "新建顶层章节",
          attrs: { "aria-label": "新建顶层章节" },
        }),
      ]),
    ]),
    list,
    element("div", { className: "sidebar-stats" }, [
      element("span", { text: "有效字符" }),
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
    setNotice("success", successMessage);
    renderWorkspace();
    scheduleSummaryRefresh();
  } catch (error) {
    setNotice("error", normalizeHostError(error).message);
    await loadWorkspace().catch(() => renderWorkspace());
  }
}

async function renameDocument(document) {
  const title = window.prompt("新的章节标题", document.title);
  if (title === null || title.trim() === document.title) return;
  await commitDocumentMutation(
    "rename_document",
    { documentId: document.id, expectedRevision: document.revision, title },
    document.id,
    `已重命名为“${title.trim()}”，并记录版本。`,
  );
}

async function reorderDocument(document, direction) {
  await commitDocumentMutation(
    "reorder_document",
    { documentId: document.id, expectedRevision: document.revision, direction },
    document.id,
    `已${direction === "up" ? "上移" : "下移"}“${document.title}”，并记录版本。`,
  );
}

async function changeDocumentDepth(document, direction) {
  await commitDocumentMutation(
    "change_document_depth",
    { documentId: document.id, expectedRevision: document.revision, direction },
    document.id,
    direction === "indent"
      ? `已将“${document.title}”缩进为上一章节的子章节。`
      : `已将“${document.title}”移出到上一层。`,
  );
}

async function archiveDocument(document) {
  const descendants = documentDescendantCount(document.id, state.workspace.documents);
  const cascade = descendants ? `及其 ${descendants} 个子章节` : "";
  if (!window.confirm(`归档“${document.title}”${cascade}？正文会被保留，并可从侧栏恢复。`)) return;
  await commitDocumentMutation(
    "set_document_archived",
    { documentId: document.id, expectedRevision: document.revision, archived: true },
    state.selectedDocumentId === document.id ? null : state.selectedDocumentId,
    `已归档“${document.title}”，正文和版本记录仍被保留。`,
  );
}

async function restoreArchivedDocument(document) {
  await commitDocumentMutation(
    "set_document_archived",
    { documentId: document.id, expectedRevision: document.revision, archived: false },
    document.id,
    `已恢复“${document.title}”。`,
  );
}

async function createDocument(parentDocumentId = null) {
  const parent = state.workspace.documents.find((document) => document.id === parentDocumentId);
  const title = window.prompt(parent ? `新建“${parent.title}”的子章节` : "新章节标题");
  if (title === null) return;
  try {
    await flushAll();
    const created = await invokeHost("create_document", {
      input: { schemaVersion: 1, title, initialText: "", parentDocumentId },
    });
    state.summaryInvalidations = await invokeHost("list_summary_invalidations");
    applyCreatedDocument(created, `已创建章节“${created.document.title}”并记录版本。`);
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
    attrs: { accept: ".md,.markdown,text/markdown,text/plain", "aria-label": "选择 Markdown 文件" },
  });
  picker.hidden = true;
  picker.addEventListener("change", async () => {
    const file = picker.files?.[0];
    picker.remove();
    if (!file) return;
    try {
      if (file.size > 2 * 1024 * 1024) {
        throw { code: "IMPORT_TOO_LARGE", message: "Markdown 文件不能超过 2 MiB。" };
      }
      await flushAll();
      const text = (await file.text()).replace(/^\uFEFF/, "");
      const title = file.name.replace(/\.(?:md|markdown|txt)$/i, "") || "导入文档";
      const created = await invokeHost("create_document", {
        input: { schemaVersion: 1, title, initialText: text },
      });
      state.summaryInvalidations = await invokeHost("list_summary_invalidations");
      applyCreatedDocument(created, `已导入 ${file.name}；原始 Markdown 已作为版本化正文保存。`);
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
    setNotice("success", `已导出 ${exported.documents} 个文档到 ${exported.path}`);
    renderWorkspace();
  } catch (error) {
    setNotice("error", normalizeHostError(error).message);
    renderWorkspace();
  }
}

function editorPane() {
  const document = state.workspace.documents.find((item) => item.id === state.selectedDocumentId);
  if (!document) {
    return element("section", { className: "editor-empty", text: "项目中还没有可编辑文档。" });
  }
  const toolbar = element("div", { className: "editor-toolbar" }, [
    element("span", { className: "toolbar-label", text: "AI 操作" }),
    ...AI_OPERATION_COMMANDS.map((command) =>
      button(command.label, "tool-button", () => runAiOperation(command.type), {
        disabled: Boolean(state.aiRunning || state.aiReview?.kind === "patch_proposal"),
        title: `${command.description} · ${command.shortcut}`,
        attrs: { "aria-keyshortcuts": command.shortcut },
      }),
    ),
    button("固定风格", "tool-button style-pin-button", pinCurrentStyleSample, {
      disabled: Boolean(state.aiRunning || state.aiReview?.kind === "patch_proposal"),
      title: "将当前选区固定为项目风格样本",
    }),
    button("命令", "tool-button command-palette-trigger", openCommandPalette, {
      title: "打开 AI 命令面板 · Ctrl/⌘+Shift+P",
      attrs: { "aria-keyshortcuts": "Control+Shift+P Meta+Shift+P" },
    }),
    state.aiRunning
      ? button("取消", "danger-button", cancelAiOperation)
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
    element("strong", { text: "检测到并发修改，草稿尚未丢失" }),
    element("p", { text: "已重新载入磁盘上的最新版本。你可以把本地草稿重新保存，或放弃它。" }),
    element("div", { className: "conflict-actions" }, [
      button("用本地草稿覆盖最新版本", "danger-button", retryConflictDraft),
      button("放弃本地草稿", "quiet-button", () => {
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
    element("span", { text: `r${block.revision}` }),
  ]);
  const content = element("div", {
    className: "block-content",
    text: block.plainText,
    attrs: {
      "data-block-id": block.id,
      "data-placeholder": editable ? "开始写作…" : "此 Block 暂不支持直接编辑",
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

function openAiContextMenu(x, y, blockId) {
  state.commandPaletteOpen = false;
  state.aiContextMenu = { x, y, blockId };
  renderWorkspace();
  queueMicrotask(() => document.querySelector(".ai-context-menu button:not(:disabled)")?.focus());
}

function aiContextMenuView() {
  const context = state.aiContextMenu;
  if (!context || !state.workspace) return null;
  const menu = element("div", {
    className: "ai-context-menu",
    attrs: { role: "menu", "aria-label": "AI 文本操作" },
  }, [
    element("div", { className: "context-menu-heading", text: "AI 文本操作" }),
    ...AI_OPERATION_COMMANDS.map((command) => commandMenuButton(command)),
    element("div", { className: "context-menu-separator", attrs: { role: "separator" } }),
    commandMenuButton({
      type: "pin_style",
      label: "固定为风格样本",
      description: "保存当前选区",
      shortcut: "",
    }),
    element("div", { className: "context-menu-hint", text: "Shift + 右键打开系统菜单" }),
  ]);
  menu.style.left = `${Math.max(8, Math.min(context.x, window.innerWidth - 244))}px`;
  menu.style.top = `${Math.max(8, Math.min(context.y, window.innerHeight - 340))}px`;
  menu.addEventListener("keydown", (event) => {
    if (event.key === "Escape") {
      event.preventDefault();
      closeCommandSurfaces();
      return;
    }
    if (!["ArrowDown", "ArrowUp"].includes(event.key)) return;
    event.preventDefault();
    const items = [...menu.querySelectorAll('button[role="menuitem"]:not(:disabled)')];
    const current = items.indexOf(document.activeElement);
    const delta = event.key === "ArrowDown" ? 1 : -1;
    items[(current + delta + items.length) % items.length]?.focus();
  });
  return menu;
}

function commandMenuButton(command) {
  const item = button("", "context-menu-command", () => executeRegisteredCommand(command.type), {
    disabled: operationCommandsDisabled(),
    attrs: {
      role: "menuitem",
      ...(command.shortcut ? { "aria-keyshortcuts": command.shortcut } : {}),
    },
  });
  item.append(element("span", { text: command.label }));
  if (command.shortcut) item.append(element("kbd", { text: command.shortcut }));
  return item;
}

function openCommandPalette() {
  if (!state.workspace || state.aiContextPreview) return;
  state.aiContextMenu = null;
  state.commandPaletteOpen = true;
  renderWorkspace();
  queueMicrotask(() => document.querySelector("#command-palette-query")?.focus());
}

function commandPaletteModal() {
  if (!state.commandPaletteOpen || !state.workspace) return null;
  const commandRows = [
    ...AI_OPERATION_COMMANDS,
    {
      type: "pin_style",
      label: "固定为风格样本",
      description: "将当前选区保存到项目风格库",
      shortcut: "",
    },
  ].map((command) => {
    const row = button("", "command-palette-item", () => executeRegisteredCommand(command.type), {
      disabled: operationCommandsDisabled(),
      attrs: {
        "data-command-search": `${command.label} ${command.description} ${command.type}`.toLowerCase(),
      },
    });
    row.append(element("span", { className: "command-palette-copy" }, [
      element("strong", { text: command.label }),
      element("span", { text: command.description }),
    ]));
    if (command.shortcut) row.append(element("kbd", { text: command.shortcut }));
    return row;
  });
  const query = element("input", {
    type: "search",
    placeholder: "搜索续写、润色、压缩…",
    attrs: {
      id: "command-palette-query",
      "aria-label": "搜索 AI 命令",
      autocomplete: "off",
    },
  });
  query.addEventListener("input", () => {
    const needle = query.value.trim().toLowerCase();
    for (const row of commandRows) {
      row.hidden = !row.dataset.commandSearch.includes(needle);
    }
  });
  query.addEventListener("keydown", (event) => {
    if (event.key === "Escape") {
      event.preventDefault();
      closeCommandSurfaces();
    } else if (event.key === "Enter") {
      event.preventDefault();
      commandRows.find((row) => !row.hidden && !row.disabled)?.click();
    }
  });
  const backdrop = element("div", {
    className: "command-palette-backdrop",
    attrs: { role: "presentation" },
  }, [
    element("section", {
      className: "command-palette",
      attrs: { role: "dialog", "aria-modal": "true", "aria-labelledby": "command-palette-title" },
    }, [
      element("div", { className: "command-palette-heading" }, [
        element("div", {}, [
          element("span", { className: "eyebrow", text: "COMMAND PALETTE" }),
          element("h2", { text: "选择 AI 操作", attrs: { id: "command-palette-title" } }),
        ]),
        element("kbd", { text: "Esc" }),
      ]),
      query,
      operationCommandsDisabled()
        ? element("p", { className: "command-palette-warning", text: "请先完成或放弃当前 AI 操作与审查。" })
        : null,
      element("div", { className: "command-palette-list" }, commandRows),
    ]),
  ]);
  backdrop.addEventListener("pointerdown", (event) => {
    if (event.target === backdrop) closeCommandSurfaces();
  });
  return backdrop;
}

function executeRegisteredCommand(commandType) {
  closeCommandSurfaces();
  if (commandType === "pin_style") {
    void pinCurrentStyleSample();
  } else {
    void runAiOperation(commandType);
  }
}

function closeCommandSurfaces() {
  state.aiContextMenu = null;
  state.commandPaletteOpen = false;
  document.querySelector(".ai-context-menu")?.remove();
  document.querySelector(".command-palette-backdrop")?.remove();
}

async function runAiOperation(operationType) {
  if (state.aiRunning || state.aiReview?.kind === "patch_proposal") return;
  const providerSettings = state.providerSettings[state.selectedProviderId];
  const credentialRequired = providerRequiresCredential(state.selectedProviderId);
  if (!providerSettings.enabled || (credentialRequired && !providerSettings.credentialExists)) {
    state.providersOpen = true;
    state.versionsOpen = false;
    state.stylesOpen = false;
    state.candidatesOpen = false;
    setNotice(
      "warning",
      credentialRequired
        ? `请先在模型设置中启用 ${PROVIDER_PRESETS[state.selectedProviderId].label} 并保存 API Key。`
        : `请先在模型设置中启用 ${PROVIDER_PRESETS[state.selectedProviderId].label}。`,
    );
    renderWorkspace();
    return;
  }
  try {
    await flushAll();
    const documentBlocks = blocksForDocument(state.workspace, state.selectedDocumentId);
    const block = state.workspace.blocks.find((candidate) => candidate.id === state.activeBlockId)
      ?? documentBlocks.find(isEditableBlock);
    if (!block || !isEditableBlock(block)) {
      throw { code: "TARGET_INVALID", message: "请先把光标放到一个可编辑文本 Block 中。" };
    }
    const selection = state.aiSelection?.blockId === block.id ? state.aiSelection : null;
    let from = Math.min(selection?.from ?? 0, block.plainText.length);
    let to = Math.min(selection?.to ?? block.plainText.length, block.plainText.length);
    if (operationType === "continue_scene") {
      const caret = selection ? selection.to : block.plainText.length;
      from = caret;
      to = caret;
    } else if (from === to) {
      from = 0;
      to = block.plainText.length;
    }
    const Channel = window.__TAURI__?.core?.Channel;
    if (typeof Channel !== "function") {
      throw { code: "HOST_UNAVAILABLE", message: "当前桌面运行时不支持流式模型通道。" };
    }
    const controller = new AbortController();
    state.aiRunning = {
      controller,
      label: "正在编译上下文…",
      received: 0,
    };
    state.aiReview = null;
    setNotice(null, null);
    updateAiStatus(state.aiRunning.label);
    const execution = await runDesktopOperation({
      invokeHost,
      createChannel: () => new Channel(),
      providerSettings,
      workspace: state.workspace,
      block,
      from,
      to,
      operationType,
      loadOperationContext: (input) => invokeHost("get_operation_context", { input }),
      signal: controller.signal,
      onProgress: updateAiProgress,
      confirmContext: confirmCompiledContext,
    });
    if (execution.result.kind === "patch_proposal") {
      state.aiReview = {
        kind: "patch_proposal",
        intent: execution.intent,
        result: execution.result,
        session: await createDesktopReview(execution.result.proposal),
        candidateBranch: null,
        busy: false,
      };
      await refreshReviewCandidates();
      setNotice("success", `AI 已生成 ${execution.result.proposal.hunks.length} 个可审查修改，正文尚未改变。`);
    } else {
      state.aiReview = {
        kind: "findings",
        intent: execution.intent,
        result: execution.result,
      };
      setNotice("success", `批评完成，共 ${execution.result.findings.length} 条发现。`);
    }
  } catch (error) {
    const normalized = normalizeHostError(error);
    setNotice(normalized.code === "OPERATION_CANCELLED" ? "warning" : "error", normalized.message);
  } finally {
    state.aiContextPreview = null;
    state.aiRunning = null;
    renderWorkspace();
  }
}

function confirmCompiledContext(packet) {
  return new Promise((resolve, reject) => {
    state.aiContextPreview = { packet, resolve, reject };
    if (state.aiRunning) state.aiRunning.label = "等待确认发送上下文…";
    renderWorkspace();
  });
}

function contextPreviewModal() {
  const preview = state.aiContextPreview;
  if (!preview) return null;
  const { packet } = preview;
  return element("div", { className: "modal-backdrop", attrs: { role: "presentation" } }, [
    element("section", {
      className: "context-modal",
      attrs: { role: "dialog", "aria-modal": "true", "aria-labelledby": "context-title" },
    }, [
      element("div", { className: "review-heading" }, [
        element("div", {}, [
          element("span", { className: "eyebrow", text: "CONTEXT PACKET" }),
          element("h2", { text: "确认即将发送的上下文", attrs: { id: "context-title" } }),
        ]),
        element("span", {
          className: "count-pill",
          text: `约 ${packet.budget.estimatedInput.toLocaleString("zh-CN")} tokens`,
        }),
      ]),
      element("p", {
        className: "context-explainer",
        text: "只有下列内容会发送到当前云端模型。API Key、废弃内容和被策略拒绝的来源不会包含在请求中。",
      }),
      element("div", { className: "context-items" }, packet.items.map((item) => element("article", {
        className: "context-item",
      }, [
        element("div", { className: "context-item-heading" }, [
          element("strong", { text: item.tier }),
          element("code", { text: item.sourceRef }),
          item.mandatory ? element("span", { className: "mandatory-pill", text: "必需" }) : null,
        ]),
        element("pre", { text: item.content || "（空选区：插入点）" }),
        element("span", { className: "context-reason", text: item.reasonCodes.join(" · ") }),
      ]))),
      packet.exclusions.length
        ? element("details", { className: "context-exclusions" }, [
            element("summary", { text: `${packet.exclusions.length} 个来源未发送` }),
            ...packet.exclusions.map((item) => element("code", {
              text: `${item.sourceRef} · ${item.reason}`,
            })),
          ])
        : null,
      element("div", { className: "modal-actions" }, [
        button("取消本次操作", "quiet-button", cancelContextPreview),
        button("确认并发送", "primary-button context-confirm", approveContextPreview),
      ]),
    ]),
  ]);
}

function approveContextPreview() {
  const preview = state.aiContextPreview;
  if (!preview) return;
  state.aiContextPreview = null;
  renderWorkspace();
  preview.resolve();
  updateAiStatus("模型正在生成…");
}

function cancelContextPreview() {
  const preview = state.aiContextPreview;
  if (!preview) return;
  state.aiContextPreview = null;
  state.aiRunning?.controller.abort();
  preview.reject(new Error("Context transmission was cancelled"));
}

function updateAiProgress(event) {
  if (!state.aiRunning) return;
  if (event.type === "lifecycle") {
    const labels = {
      compiling: "正在编译最小充分上下文…",
      preflight: "正在校验目标与上下文…",
      queued: "请求已进入队列…",
      streaming: "模型正在生成…",
      validating: "正在校验模型输出…",
      review: "正在生成修改提案…",
    };
    state.aiRunning.label = labels[event.transition.to] ?? state.aiRunning.label;
  } else if (event.type === "model_text_delta") {
    state.aiRunning.received += event.text.length;
    state.aiRunning.label = `模型正在生成… ${state.aiRunning.received.toLocaleString("zh-CN")} 字符`;
  } else if (event.type === "model_reasoning_delta") {
    state.aiRunning.label = "模型正在推理…";
  }
  updateAiStatus(state.aiRunning.label);
}

function updateAiStatus(label) {
  const status = document.querySelector("#ai-status");
  if (status) status.textContent = label;
}

function cancelAiOperation() {
  if (state.aiContextPreview) {
    cancelContextPreview();
    return;
  }
  state.aiRunning?.controller.abort();
  updateAiStatus("正在取消…");
}

function aiReviewView() {
  const review = state.aiReview;
  if (!review) return null;
  if (review.kind === "findings") {
    return element("section", { className: "ai-review-card findings-card" }, [
      element("div", { className: "review-heading" }, [
        element("div", {}, [
          element("span", { className: "eyebrow", text: "AI FINDINGS" }),
          element("h2", { text: "批评与检查结果" }),
        ]),
        button("关闭", "quiet-button", () => {
          state.aiReview = null;
          renderWorkspace();
        }),
      ]),
      review.result.summary ? element("p", { className: "review-summary", text: review.result.summary }) : null,
      ...review.result.findings.map((finding) => element("article", {
        className: `finding finding-${finding.severity}`,
      }, [
        element("strong", { text: finding.severity.toUpperCase() }),
        element("p", { text: finding.message }),
        finding.sourceRef ? element("code", { text: finding.sourceRef }) : null,
      ])),
    ]);
  }

  const proposal = review.result.proposal;
  const accepted = Object.values(review.session.decisions).filter((value) => value === "accepted").length;
  const rejected = Object.values(review.session.decisions).filter((value) => value === "rejected").length;
  const total = proposal.hunks.length;
  return element("section", { className: "ai-review-card" }, [
    element("div", { className: "review-heading" }, [
      element("div", {}, [
        element("span", { className: "eyebrow", text: "PATCH REVIEW" }),
        element("h2", { text: "逐项审查 AI 修改" }),
      ]),
      element("div", { className: "review-heading-actions" }, [
        element("span", { className: "count-pill", text: `${accepted} 接受 · ${rejected} 拒绝` }),
        element("div", { className: "review-batch-actions", attrs: { "aria-label": "批量审查" } }, [
          button("全部接受", "small-button", () => decideAllCurrentHunks("accepted"), {
            disabled: review.busy || accepted === total,
          }),
          button("全部拒绝", "quiet-button", () => decideAllCurrentHunks("rejected"), {
            disabled: review.busy || rejected === total,
          }),
        ]),
      ]),
    ]),
    proposal.summary ? element("p", { className: "review-summary", text: proposal.summary }) : null,
    review.candidateBranch
      ? element("div", { className: "review-branch-notice" }, [
          element("span", { text: "已保存候选分支" }),
          element("code", { text: review.candidateBranch.branchName }),
        ])
      : null,
    ...proposal.hunks.map((hunk, index) => hunkReviewView(review, hunk, index)),
    element("div", { className: "review-footer" }, [
      element("div", { className: "review-footer-group" }, [
        button("稍后审查", "quiet-button", deferCurrentReview, { disabled: review.busy }),
        button("放弃提案", "quiet-button", rejectCurrentReview, { disabled: review.busy }),
      ]),
      element("div", { className: "review-footer-group" }, [
        button(
          review.candidateBranch ? "已保存分支" : "保存为分支",
          "secondary-button",
          createCurrentCandidateBranch,
          {
            disabled: review.busy
              || review.session.status !== "ready"
              || accepted === 0
              || Boolean(review.candidateBranch),
            title: accepted === 0
              ? "至少接受一个修改项后才能建立候选分支"
              : "创建独立快照分支，不改动当前正文",
          },
        ),
        button(accepted ? "应用已接受修改" : "完成审查（不改正文）", "primary-button review-apply", applyCurrentReview, {
          disabled: review.busy || review.session.status !== "ready",
          title: review.session.status === "ready" ? "创建 ai_accept Commit 并完成审计" : "请先处理全部修改项",
        }),
      ]),
    ]),
  ]);
}

function hunkReviewView(review, hunk, index) {
  const decision = review.session.decisions[hunk.id];
  return element("article", { className: `review-hunk decision-${decision}` }, [
    element("div", { className: "hunk-heading" }, [
      element("strong", { text: `修改 ${index + 1}` }),
      element("span", { text: hunk.granularity }),
    ]),
    element("div", { className: "hunk-diff" }, [
      element("del", { text: hunk.original || "∅" }),
      element("ins", { text: hunk.replacement || "∅" }),
    ]),
    element("div", { className: "hunk-actions" }, [
      button(decision === "accepted" ? "已接受" : "接受", "small-button", () => decideCurrentHunk(hunk.id, "accepted"), {
        disabled: review.busy || decision === "accepted",
      }),
      button(decision === "rejected" ? "已拒绝" : "拒绝", "quiet-button", () => decideCurrentHunk(hunk.id, "rejected"), {
        disabled: review.busy || decision === "rejected",
      }),
    ]),
  ]);
}

async function decideCurrentHunk(hunkId, decision) {
  const review = state.aiReview;
  if (review?.kind !== "patch_proposal" || review.busy) return;
  review.busy = true;
  renderWorkspace();
  try {
    const previous = review.session;
    const next = decideDesktopHunk(review.result.proposal, previous, hunkId, decision);
    await persistDesktopReviewDecision({
      invokeHost,
      proposalId: review.result.proposal.id,
      previous,
      next,
      hunkId,
      decision,
    });
    review.session = next;
    await refreshReviewCandidates();
  } catch (error) {
    setNotice("error", normalizeHostError(error).message);
  } finally {
    review.busy = false;
    renderWorkspace();
  }
}

async function decideAllCurrentHunks(decision) {
  const review = state.aiReview;
  if (review?.kind !== "patch_proposal" || review.busy) return;
  review.busy = true;
  renderWorkspace();
  try {
    const plan = planDesktopReviewDecisions(review.result.proposal, review.session, decision);
    for (const step of plan.steps) {
      await persistDesktopReviewDecision({
        invokeHost,
        proposalId: review.result.proposal.id,
        previous: step.previous,
        next: step.next,
        hunkId: step.hunkId,
        decision: step.decision,
      });
      review.session = step.next;
    }
    await refreshReviewCandidates();
    setNotice(
      "success",
      decision === "accepted"
        ? `已接受全部 ${review.result.proposal.hunks.length} 个修改项；应用前正文仍未改变。`
        : `已拒绝全部 ${review.result.proposal.hunks.length} 个修改项；完成审查后正文不会改变。`,
    );
  } catch (error) {
    setNotice("error", normalizeHostError(error).message);
  } finally {
    review.busy = false;
    renderWorkspace();
  }
}

async function deferCurrentReview() {
  const review = state.aiReview;
  if (review?.kind !== "patch_proposal" || review.busy) return;
  state.aiReview = null;
  state.candidatesOpen = true;
  try {
    await refreshReviewCandidates();
    setNotice("success", "候选已保留，可从候选中心或重新打开项目后继续审查。");
  } catch (error) {
    setNotice("error", normalizeHostError(error).message);
  }
  renderWorkspace();
}

async function createCurrentCandidateBranch() {
  const review = state.aiReview;
  if (
    review?.kind !== "patch_proposal"
    || review.busy
    || review.session.status !== "ready"
    || review.candidateBranch
    || !Object.values(review.session.decisions).includes("accepted")
  ) return;
  const suggested = "AI 候选 " + new Intl.DateTimeFormat("zh-CN", {
    month: "2-digit",
    day: "2-digit",
    hour: "2-digit",
    minute: "2-digit",
  }).format(new Date());
  const branchName = window.prompt("候选分支名称", suggested)?.trim();
  if (!branchName) return;
  if (branchName.length > 120) {
    setNotice("error", "候选分支名称不能超过 120 个字符。");
    renderWorkspace();
    return;
  }
  review.busy = true;
  renderWorkspace();
  try {
    const response = await invokeHost("create_review_candidate_branch", {
      input: {
        schemaVersion: 1,
        proposalId: review.result.proposal.id,
        expectedReviewRevision: review.session.revision,
        branchName,
      },
    });
    review.candidateBranch = response.branch;
    await refreshReviewCandidates();
    setNotice(
      "success",
      "已创建候选分支“" + response.branch.branchName + "”；当前正文与主分支未改变。",
    );
  } catch (error) {
    setNotice("error", normalizeHostError(error).message);
  } finally {
    review.busy = false;
    renderWorkspace();
  }
}

async function rejectCurrentReview() {
  const review = state.aiReview;
  if (review?.kind !== "patch_proposal" || review.busy) return;
  review.busy = true;
  renderWorkspace();
  try {
    await rejectDesktopReview({
      invokeHost,
      proposalId: review.result.proposal.id,
      session: review.session,
    });
    state.aiReview = null;
    await refreshReviewCandidates();
    setNotice("success", "提案已拒绝，正文未发生变化，审计记录已保留。 ");
  } catch (error) {
    setNotice("error", normalizeHostError(error).message);
    review.busy = false;
  }
  renderWorkspace();
}

async function applyCurrentReview() {
  const review = state.aiReview;
  if (review?.kind !== "patch_proposal" || review.busy || review.session.status !== "ready") return;
  review.busy = true;
  renderWorkspace();
  try {
    const compilation = await compileDesktopReview({
      proposal: review.result.proposal,
      session: review.session,
      workspace: state.workspace,
    });
    if (compilation.status === "rejected") {
      await rejectDesktopReview({
        invokeHost,
        proposalId: review.result.proposal.id,
        session: review.session,
      });
      state.aiReview = null;
      await refreshReviewCandidates();
      setNotice("success", "所有修改项均已拒绝，正文未发生变化。 ");
      renderWorkspace();
      return;
    }
    if (compilation.status !== "ready_to_apply") {
      throw { code: "CONFLICT", message: "提案目标已变化，无法安全应用。请重新发起 AI 操作。" };
    }
    const response = await invokeHost("apply_reviewed_proposal", {
      input: {
        schemaVersion: 1,
        proposalId: review.result.proposal.id,
        expectedReviewRevision: review.session.revision,
      },
    });
    state.workspace = applySaveResponse(state.workspace, response.save);
    state.summaryInvalidations = await invokeHost("list_summary_invalidations");
    state.session = {
      ...state.session,
      project: {
        ...state.session.project,
        headCommitId: response.save.headCommitId,
        revision: response.save.projectRevision,
      },
    };
    state.aiReview = null;
    state.versionHistory = null;
    await refreshReviewCandidates();
    setNotice("success", `已应用 ${response.acceptedHunks} 个修改项，并创建可审计的 ai_accept Commit。`);
    scheduleSummaryRefresh();
  } catch (error) {
    setNotice("error", normalizeHostError(error).message);
    if (state.aiReview) state.aiReview.busy = false;
  }
  renderWorkspace();
}

function updateSaveStatus(status) {
  state.saveStatus = status;
  const badge = document.querySelector("#save-status");
  if (badge) badge.textContent = status;
}

function summaryStatusLabel() {
  if (state.summaryRefreshBusy) return `摘要更新中 · ${state.summaryInvalidations.length}`;
  return state.summaryInvalidations.length
    ? `摘要待更新 ${state.summaryInvalidations.length}`
    : "摘要已就绪";
}

function updateSummaryStatus() {
  const badge = document.querySelector("#summary-status");
  if (badge) badge.textContent = summaryStatusLabel();
}

function scheduleSummaryRefresh(delay = SUMMARY_REFRESH_DEBOUNCE_MS) {
  clearTimeout(state.summaryRefreshTimer);
  state.summaryRefreshTimer = null;
  if (!state.session?.isOpen || !state.workspace || !state.summaryInvalidations.length || state.summaryRefreshBusy) {
    updateSummaryStatus();
    return;
  }
  state.summaryRefreshTimer = setTimeout(runSummaryRefresh, delay);
}

async function runSummaryRefresh() {
  state.summaryRefreshTimer = null;
  if (!state.session?.isOpen || !state.workspace || state.summaryRefreshBusy) return;
  state.summaryRefreshBusy = true;
  updateSummaryStatus();
  let failed = false;
  try {
    const report = await invokeHost("refresh_summaries", {
      input: { schemaVersion: 1, maxItems: 12 },
    });
    state.summaryInvalidations = await invokeHost("list_summary_invalidations");
    if (report.remaining !== state.summaryInvalidations.length) {
      throw new Error("Host summary refresh report does not match the invalidation queue");
    }
    state.summaryRefreshFailures = 0;
  } catch (error) {
    failed = true;
    state.summaryRefreshFailures += 1;
    console.warn("Background summary refresh failed", normalizeHostError(error));
  } finally {
    state.summaryRefreshBusy = false;
    updateSummaryStatus();
    if (state.summaryInvalidations.length) {
      const retryDelay = failed
        ? Math.min(30_000, 1_000 * (2 ** Math.min(state.summaryRefreshFailures, 5)))
        : 250;
      scheduleSummaryRefresh(retryDelay);
    }
  }
}

function queueSave(blockId, plainText, immediate = false) {
  const block = state.workspace.blocks.find((item) => item.id === blockId);
  if (!block) return;
  if (plainText === block.plainText && !state.savePromises.has(blockId)) {
    state.pendingText.delete(blockId);
    updateSaveStatus("已保存");
    return;
  }
  state.pendingText.set(blockId, plainText);
  updateSaveStatus("编辑中");
  clearTimeout(state.saveTimers.get(blockId));
  const timer = setTimeout(() => flushBlock(blockId), immediate ? 0 : SAVE_DEBOUNCE_MS);
  state.saveTimers.set(blockId, timer);
}

function flushBlock(blockId) {
  clearTimeout(state.saveTimers.get(blockId));
  state.saveTimers.delete(blockId);
  const active = state.savePromises.get(blockId);
  if (active) return active.then(() => flushBlock(blockId));
  if (!state.pendingText.has(blockId)) return Promise.resolve();
  const plainText = state.pendingText.get(blockId);
  state.pendingText.delete(blockId);
  const block = state.workspace.blocks.find((item) => item.id === blockId);
  if (!block) return Promise.resolve();
  updateSaveStatus("保存中…");
  const promise = invokeHost("save_block", { input: buildSaveBlockRequest(block, plainText) })
    .then(async (response) => {
      state.workspace = applySaveResponse(state.workspace, response);
      state.summaryInvalidations = await invokeHost("list_summary_invalidations");
      state.session = {
        ...state.session,
        project: { ...state.session.project, headCommitId: response.headCommitId, revision: response.projectRevision },
      };
      updateSaveStatus("已保存");
      scheduleSummaryRefresh();
    })
    .catch(async (error) => {
      const normalized = normalizeHostError(error);
      if (normalized.code === "NO_CHANGES") {
        updateSaveStatus("已保存");
        return;
      }
      if (normalized.code === "CONFLICT") {
        state.conflictDraft = { blockId, plainText };
        setNotice("warning", "检测到其他提交，已保留本地草稿并重新载入最新版本。");
        await loadWorkspace();
        updateSaveStatus("存在冲突");
        return;
      }
      state.pendingText.set(blockId, plainText);
      setNotice("error", normalized.message);
      updateSaveStatus("保存失败");
      renderWorkspace();
    })
    .finally(() => state.savePromises.delete(blockId))
    .then(() => (state.pendingText.has(blockId) ? flushBlock(blockId) : undefined));
  state.savePromises.set(blockId, promise);
  return promise;
}

async function flushAll({ allowConflict = false } = {}) {
  for (const [blockId, timer] of state.saveTimers) {
    clearTimeout(timer);
    state.saveTimers.delete(blockId);
  }
  const ids = new Set([...state.pendingText.keys(), ...state.savePromises.keys()]);
  await Promise.all([...ids].map((blockId) => flushBlock(blockId)));
  if (state.conflictDraft && !allowConflict) {
    throw { code: "CONFLICT", message: "请先处理已保留的冲突草稿，再继续此操作。" };
  }
}

function retryConflictDraft() {
  const conflict = state.conflictDraft;
  if (!conflict) return;
  state.conflictDraft = null;
  queueSave(conflict.blockId, conflict.plainText, true);
  renderWorkspace();
}

async function createCheckpoint() {
  try {
    await flushAll();
    const checkpoint = await invokeHost("create_checkpoint");
    setNotice("success", `已建立检查点 ${checkpoint.id.slice(0, 20)}…`);
    state.versionHistory = null;
    if (state.versionsOpen) await loadVersionHistory();
    else renderWorkspace();
  } catch (error) {
    setNotice("error", normalizeHostError(error).message);
    renderWorkspace();
  }
}

async function refreshReviewCandidates({ render = false } = {}) {
  state.reviewCandidates = await invokeHost("list_review_candidates");
  if (render) renderWorkspace();
}

async function toggleCandidates() {
  state.candidatesOpen = !state.candidatesOpen;
  if (state.candidatesOpen) {
    state.providersOpen = false;
    state.stylesOpen = false;
    state.versionsOpen = false;
    try {
      await refreshReviewCandidates();
    } catch (error) {
      state.candidatesOpen = false;
      setNotice("error", normalizeHostError(error).message);
    }
  }
  renderWorkspace();
}

function candidateDrawer() {
  const active = state.reviewCandidates.filter(
    (candidate) => candidate.status === "review" || candidate.status === "ready" || candidate.status === "conflicted",
  );
  const history = state.reviewCandidates.filter(
    (candidate) => candidate.status === "applied" || candidate.status === "rejected",
  );
  const drawer = element("aside", { className: "version-drawer candidate-drawer" }, [
    element("div", { className: "drawer-heading" }, [
      element("div", {}, [
        element("span", { className: "eyebrow", text: "PATCH CANDIDATES" }),
        element("h2", { text: "候选中心" }),
      ]),
      button("×", "icon-button", toggleCandidates, { title: "关闭候选中心" }),
    ]),
    element("p", {
      className: "drawer-intro",
      text: "提案与审查决定保存在项目数据库中；稍后或重新打开项目仍可继续。",
    }),
    element("h3", { text: "待处理 · " + active.length }),
  ]);
  if (!active.length) {
    drawer.append(element("p", { className: "drawer-empty", text: "没有待处理候选。" }));
  } else {
    drawer.append(...active.map(reviewCandidateCard));
  }
  if (history.length) {
    drawer.append(element("h3", { text: "已完成 · " + history.length }));
    drawer.append(...history.slice(0, 30).map(reviewCandidateCard));
  }
  return drawer;
}

function reviewCandidateCard(candidate) {
  const document = state.workspace.documents.find((item) => item.id === candidate.targetDocumentId);
  const isCurrent = state.aiReview?.kind === "patch_proposal"
    && state.aiReview.result.proposal.id === candidate.proposalId;
  const canResume = candidate.status === "review" || candidate.status === "ready";
  const busy = state.candidateBusy === candidate.proposalId;
  const card = element("article", { className: "candidate-card" }, [
    element("div", { className: "candidate-card-heading" }, [
      element("span", {
        className: "candidate-status status-" + candidate.status,
        text: reviewCandidateStatusLabel(candidate.status),
      }),
      element("span", { text: candidate.hunkCount + " 项" }),
    ]),
    element("strong", { text: candidate.summary || (document ? document.title : "AI 修改候选") }),
    element("span", { text: candidate.providerId + " · " + candidate.model }),
    element("span", { text: formatDate(candidate.updatedAt) }),
    candidate.candidateBranch
      ? element("div", { className: "candidate-branch-badge" }, [
          element("span", { text: "分支" }),
          element("code", { text: candidate.candidateBranch.branchName }),
        ])
      : null,
  ]);
  if (canResume) {
    card.append(button(
      isCurrent ? "正在审查" : busy ? "正在恢复…" : "继续审查",
      "small-button candidate-resume",
      () => openReviewCandidate(candidate.proposalId),
      { disabled: isCurrent || busy || Boolean(state.aiRunning || state.aiReview) },
    ));
  } else if (candidate.status === "conflicted") {
    card.append(element("p", {
      className: "candidate-note",
      text: "目标正文已变化；当前版本保留审计，等待后续重基工具。",
    }));
  }
  return card;
}

function reviewCandidateStatusLabel(status) {
  return {
    review: "审查中",
    ready: "可应用",
    conflicted: "有冲突",
    applied: "已应用",
    rejected: "已拒绝",
  }[status] || status;
}

async function openReviewCandidate(proposalId) {
  if (state.aiRunning || state.aiReview || state.candidateBusy) return;
  state.candidateBusy = proposalId;
  renderWorkspace();
  try {
    await flushAll();
    const detail = await invokeHost("load_review_candidate", {
      input: { schemaVersion: 1, proposalId },
    });
    const hydrated = await hydrateDesktopReviewCandidate(detail);
    if (hydrated.session.status !== "review" && hydrated.session.status !== "ready") {
      throw { code: "REVIEW_FINALIZED", message: "这个候选当前不能继续审查。" };
    }
    state.aiReview = {
      kind: "patch_proposal",
      intent: null,
      result: { kind: "patch_proposal", proposal: hydrated.proposal },
      session: hydrated.session,
      candidateBranch: hydrated.candidateBranch,
      resumed: true,
      busy: false,
    };
    if (state.workspace.documents.some((document) => document.id === hydrated.proposal.target.documentId)) {
      state.selectedDocumentId = hydrated.proposal.target.documentId;
    }
    state.activeBlockId = hydrated.proposal.target.blockId;
    state.candidatesOpen = false;
    setNotice("success", "已从项目数据库恢复未完成的候选审查。");
  } catch (error) {
    setNotice("error", normalizeHostError(error).message);
  } finally {
    state.candidateBusy = null;
    renderWorkspace();
  }
}

async function toggleVersions() {
  state.versionsOpen = !state.versionsOpen;
  if (state.versionsOpen) {
    state.providersOpen = false;
    state.stylesOpen = false;
    state.candidatesOpen = false;
  }
  if (state.versionsOpen) await loadVersionHistory();
  else renderWorkspace();
}

async function toggleProviders() {
  state.providersOpen = !state.providersOpen;
  if (state.providersOpen) {
    state.versionsOpen = false;
    state.stylesOpen = false;
    state.candidatesOpen = false;
    await refreshProviderSecretStatus();
  }
  renderWorkspace();
}

function toggleStyles() {
  state.stylesOpen = !state.stylesOpen;
  if (state.stylesOpen) {
    state.providersOpen = false;
    state.versionsOpen = false;
    state.candidatesOpen = false;
  }
  renderWorkspace();
}

function styleDrawer() {
  const active = state.styleSamples.filter((sample) => sample.status === "canonical");
  const archived = state.styleSamples.filter((sample) => sample.status === "archived");
  const canonicalKnowledge = state.knowledgeItems.filter((item) => item.status === "canonical");
  const inactiveKnowledge = state.knowledgeItems.filter((item) => item.status !== "canonical");
  const drawer = element("aside", { className: "version-drawer style-drawer" }, [
    element("div", { className: "drawer-heading" }, [
      element("div", {}, [
        element("span", { className: "eyebrow", text: "KNOWLEDGE & STYLE" }),
        element("h2", { text: "项目知识与风格" }),
      ]),
      button("×", "icon-button", toggleStyles, { title: "关闭" }),
    ]),
    element("p", {
      className: "style-help",
      text: "canonical 事实与约束进入 L3 Context；归档或拒绝内容永不召回。never_send 只允许发给本地模型。",
    }),
    knowledgeForm(),
    element("h3", { text: `有效事实 / 约束 · ${canonicalKnowledge.length}` }),
  ]);
  if (!canonicalKnowledge.length) {
    drawer.append(element("p", { className: "drawer-empty", text: "尚无有效事实或约束。" }));
  }
  for (const item of canonicalKnowledge) drawer.append(knowledgeItemCard(item));
  if (inactiveKnowledge.length) {
    drawer.append(element("h3", { text: `非 canonical · ${inactiveKnowledge.length}` }));
    for (const item of inactiveKnowledge) drawer.append(knowledgeItemCard(item));
  }
  drawer.append(element("h3", { text: `启用风格 · ${active.length}` }));
  if (!active.length) {
    drawer.append(element("p", { className: "drawer-empty", text: "尚无启用样本。请先在正文中选择一段文字。" }));
  }
  for (const sample of active) drawer.append(styleSampleCard(sample));
  if (archived.length) {
    drawer.append(element("h3", { text: `已归档 · ${archived.length}` }));
    for (const sample of archived) drawer.append(styleSampleCard(sample));
  }
  return drawer;
}

function knowledgeForm() {
  const kind = element("select", { name: "kind", attrs: { "aria-label": "知识类型" } }, [
    element("option", { value: "fact", text: "事实" }),
    element("option", { value: "constraint", text: "约束" }),
  ]);
  const severity = element("select", { name: "severity", disabled: true, attrs: { "aria-label": "约束强度" } }, [
    element("option", { value: "hard", text: "硬约束" }),
    element("option", { value: "soft", text: "软约束" }),
  ]);
  kind.addEventListener("change", () => { severity.disabled = kind.value !== "constraint"; });
  const sensitivity = element("select", { name: "sensitivity", attrs: { "aria-label": "发送策略" } }, [
    element("option", { value: "local_sensitive", text: "可在确认后发送" }),
    element("option", { value: "never_send", text: "仅本地模型" }),
    element("option", { value: "local", text: "本地内容" }),
    element("option", { value: "public", text: "公开内容" }),
  ]);
  const content = element("textarea", {
    name: "content",
    placeholder: "写入不可违背的事实，或 AI 必须遵守的约束",
    attrs: { required: "", rows: "3", maxlength: "65536" },
  });
  const form = element("form", { className: "knowledge-form" }, [
    element("div", { className: "knowledge-form-grid" }, [kind, severity, sensitivity]),
    labeledInput("标题", "title", "例如：主角视觉 / 禁止剧透", true),
    element("label", { className: "field" }, [element("span", { text: "内容" }), content]),
    element("button", { className: "primary-button", text: "添加为 canonical", type: "submit" }),
  ]);
  form.addEventListener("submit", createKnowledgeItem);
  return form;
}

function knowledgeItemCard(item) {
  const inactive = item.status !== "canonical";
  const kindLabel = item.kind === "fact"
    ? "事实"
    : (item.severity === "hard" ? "硬约束" : "软约束");
  const actions = inactive
    ? [button("恢复 canonical", "small-button", () => updateKnowledgeItemStatus(item, "canonical"))]
    : [
        button("归档", "small-button", () => updateKnowledgeItemStatus(item, "archived")),
        button("拒绝", "small-button", () => updateKnowledgeItemStatus(item, "rejected")),
      ];
  return element("article", { className: `style-card knowledge-card${inactive ? " archived" : ""}` }, [
    element("div", { className: "style-card-heading" }, [
      element("strong", { text: item.title }),
      element("span", {
        className: `policy-pill${item.sensitivity === "never_send" ? " local-only" : ""}`,
        text: `${kindLabel} · ${item.sensitivity === "never_send" ? "仅本地" : item.status}`,
      }),
    ]),
    element("p", { text: item.content }),
    element("div", { className: "style-card-footer" }, [
      element("code", { text: `r${item.revision} · ${item.authority} · ${item.contentHash.slice(0, 15)}…` }),
      element("div", { className: "knowledge-actions" }, actions),
    ]),
  ]);
}

async function createKnowledgeItem(event) {
  event.preventDefault();
  const form = event.currentTarget;
  const kind = formValue(form, "kind");
  const input = {
    schemaVersion: 1,
    kind,
    title: formValue(form, "title"),
    content: formValue(form, "content"),
    sensitivity: formValue(form, "sensitivity"),
    severity: kind === "constraint" ? formValue(form, "severity") : null,
  };
  setFormBusy(form, true);
  try {
    const created = await invokeHost("create_knowledge_item", {
      input,
    });
    state.knowledgeItems = [created, ...state.knowledgeItems];
    setNotice("success", "项目知识已设为 canonical；下次 AI 操作会在发送前列出它。" );
    renderWorkspace();
  } catch (error) {
    setNotice("error", normalizeHostError(error).message);
    setFormBusy(form, false);
  }
}

async function updateKnowledgeItemStatus(item, status) {
  try {
    const updated = await invokeHost("set_knowledge_item_status", {
      input: {
        schemaVersion: 1,
        id: item.id,
        expectedRevision: item.revision,
        status,
      },
    });
    state.knowledgeItems = state.knowledgeItems.map((candidate) => candidate.id === updated.id ? updated : candidate);
    setNotice("success", status === "canonical" ? "知识已恢复为 canonical。" : `知识已${status === "archived" ? "归档" : "拒绝"}，不会再进入上下文。`);
    renderWorkspace();
  } catch (error) {
    setNotice("error", normalizeHostError(error).message);
    state.knowledgeItems = await invokeHost("list_knowledge_items").catch(() => state.knowledgeItems);
    renderWorkspace();
  }
}

function styleSampleCard(sample) {
  const archived = sample.status === "archived";
  return element("article", { className: `style-card${archived ? " archived" : ""}` }, [
    element("div", { className: "style-card-heading" }, [
      element("strong", { text: sample.title }),
      element("span", {
        className: `policy-pill${sample.sensitivity === "never_send" ? " local-only" : ""}`,
        text: sample.sensitivity === "never_send" ? "仅本地" : "可发送",
      }),
    ]),
    element("p", { text: sample.content }),
    element("div", { className: "style-card-footer" }, [
      element("code", { text: `r${sample.revision} · ${sample.contentHash.slice(0, 15)}…` }),
      button(archived ? "重新启用" : "归档", "small-button", () => updateStyleSampleStatus(
        sample,
        archived ? "canonical" : "archived",
      )),
    ]),
  ]);
}

async function pinCurrentStyleSample() {
  try {
    await flushAll();
    const block = state.workspace.blocks.find((candidate) => candidate.id === state.activeBlockId);
    const selection = state.aiSelection?.blockId === block?.id ? state.aiSelection : null;
    if (!block || !selection || selection.from === selection.to) {
      throw { code: "STYLE_SELECTION_REQUIRED", message: "请先在一个正文 Block 中选择要固定的风格片段。" };
    }
    const content = block.plainText.slice(selection.from, selection.to).trim();
    if (!content) {
      throw { code: "STYLE_SELECTION_REQUIRED", message: "风格样本不能只有空白字符。" };
    }
    const document = state.workspace.documents.find((item) => item.id === block.documentId);
    const created = await invokeHost("create_style_sample", {
      input: {
        schemaVersion: 1,
        title: `${document?.title ?? "正文"} · 样本 ${state.styleSamples.length + 1}`,
        content,
        sensitivity: "local_sensitive",
      },
    });
    state.styleSamples = [created, ...state.styleSamples];
    state.stylesOpen = true;
    state.providersOpen = false;
    state.versionsOpen = false;
    setNotice("success", "风格样本已固定；后续 AI 操作会在发送前列出它。" );
    renderWorkspace();
  } catch (error) {
    setNotice("error", normalizeHostError(error).message);
    renderWorkspace();
  }
}

async function updateStyleSampleStatus(sample, status) {
  try {
    const updated = await invokeHost("set_style_sample_status", {
      input: {
        schemaVersion: 1,
        id: sample.id,
        expectedRevision: sample.revision,
        status,
      },
    });
    state.styleSamples = state.styleSamples.map((item) => item.id === updated.id ? updated : item);
    setNotice("success", status === "canonical" ? "风格样本已重新启用。" : "风格样本已归档，不会再进入上下文。" );
    renderWorkspace();
  } catch (error) {
    setNotice("error", normalizeHostError(error).message);
    state.styleSamples = await invokeHost("list_style_samples").catch(() => state.styleSamples);
    renderWorkspace();
  }
}

function providerDrawer() {
  const providerId = state.selectedProviderId;
  const settings = state.providerSettings[providerId];
  const credentialRequired = providerRequiresCredential(providerId);
  const drawer = element("aside", { className: "version-drawer provider-drawer" }, [
    element("div", { className: "drawer-heading" }, [
      element("div", {}, [
        element("span", { className: "eyebrow", text: "MODEL PROVIDERS" }),
        element("h2", { text: "模型与凭据" }),
      ]),
      button("×", "icon-button", toggleProviders, { title: "关闭" }),
    ]),
    element("nav", { className: "provider-tabs", attrs: { "aria-label": "模型供应商" } },
      Object.entries(PROVIDER_PRESETS).map(([id, preset]) => button(
        preset.label,
        `provider-tab${id === providerId ? " active" : ""}`,
        () => {
          state.selectedProviderId = id;
          renderWorkspace();
        },
      )),
    ),
  ]);
  const form = element("form", { className: "provider-form" }, [
    element("div", { className: `credential-status${!credentialRequired || settings.credentialExists ? " connected" : ""}` }, [
      element("span", { className: "status-dot" }),
      element("strong", {
        text: credentialRequired
          ? (settings.credentialExists ? "凭据已存入系统保险库" : "尚未保存凭据")
          : "固定本地端点 · 无需 API Key",
      }),
    ]),
    checkboxField("启用此供应商", "enabled", settings.enabled),
    ...(providerId === "ollama"
      ? ollamaProviderFields(settings)
      : [labeledInput("默认模型", "defaultModel", PROVIDER_PRESETS[providerId].defaultModel, true, settings.defaultModel)]),
    ...(providerId === "qwen" ? qwenProviderFields(settings) : []),
    ...(credentialRequired ? [
      passwordField("API Key", "apiKey", settings.credentialExists ? "留空则保持现有凭据" : "仅发送到 Rust 宿主"),
      element("p", {
        className: "form-hint",
        text: "API Key 只写入操作系统凭据库。WebView 无读取命令；本地偏好仅保存模型名、区域和启用状态。",
      }),
    ] : [
      element("p", {
        className: "form-hint",
        text: "仅连接本机回环 127.0.0.1:11434/v1；地址不可由文档或页面修改，never_send 样本可在本地上下文中使用。请先在 Ollama 中拉取同名模型。",
      }),
    ]),
    element("button", { className: "primary-button", text: "保存模型设置", type: "submit" }),
    credentialRequired && settings.credentialExists
      ? button("删除已保存凭据", "quiet-button provider-delete", deleteProviderCredential)
      : null,
  ]);
  form.addEventListener("submit", saveProviderSettings);
  drawer.append(form);
  return drawer;
}

function checkboxField(label, name, checked) {
  const input = element("input", { type: "checkbox", name });
  input.checked = checked;
  return element("label", { className: "checkbox-field" }, [input, element("span", { text: label })]);
}

function passwordField(label, name, placeholder) {
  const input = element("input", { type: "password", name, placeholder, attrs: { autocomplete: "new-password" } });
  return element("label", { className: "field" }, [element("span", { text: label }), input]);
}

function qwenProviderFields(settings) {
  const select = element("select", { name: "qwenRegion" }, [
    ["china", "中国内地"],
    ["singapore", "新加坡"],
    ["us", "美国"],
    ["germany", "德国"],
    ["japan", "日本"],
  ].map(([value, label]) => element("option", { value, text: label })));
  select.value = settings.qwenRegion;
  return [
    element("label", { className: "field" }, [element("span", { text: "部署区域" }), select]),
    labeledInput("Workspace ID（部分区域必填）", "qwenWorkspaceId", "仅允许字母、数字、_ 和 -", false, settings.qwenWorkspaceId),
  ];
}

function ollamaProviderFields(settings) {
  const discovery = state.ollamaDiscovery;
  const input = element("input", {
    name: "defaultModel",
    value: settings.defaultModel,
    placeholder: PROVIDER_PRESETS.ollama.defaultModel,
    attrs: { list: "ollama-model-list", autocomplete: "off" },
  });
  const models = discovery?.state === "success" ? discovery.models : [];
  const status = discovery?.state === "loading"
    ? "正在连接本机 Ollama…"
    : discovery?.state === "success"
      ? (models.length > 0 ? `已发现 ${models.length} 个本地模型。` : "Ollama 可达，但尚未安装模型。")
      : discovery?.state === "error"
        ? discovery.message
        : "尚未检测本机 Ollama。";
  return [
    element("label", { className: "field" }, [element("span", { text: "默认模型" }), input]),
    element("datalist", { attrs: { id: "ollama-model-list" } },
      models.map((model) => element("option", { value: model.id }))),
    button(
      discovery?.state === "loading" ? "检测中…" : "检测本地模型",
      "quiet-button",
      probeOllamaModels,
      { disabled: discovery?.state === "loading" },
    ),
    element("p", { className: "form-hint", text: status, attrs: { role: "status" } }),
  ];
}

async function probeOllamaModels(event) {
  const pendingModel = formValue(event.currentTarget.closest("form"), "defaultModel");
  state.providerSettings.ollama = {
    ...state.providerSettings.ollama,
    defaultModel: pendingModel,
  };
  state.ollamaDiscovery = { state: "loading", models: [] };
  renderWorkspace();
  try {
    const response = await invokeHost("list_ollama_models");
    const models = Array.isArray(response.models)
      ? response.models.filter((model) => model && typeof model.id === "string")
      : [];
    state.ollamaDiscovery = { state: "success", models };
  } catch (error) {
    state.ollamaDiscovery = {
      state: "error",
      models: [],
      message: `Ollama 不可用：${normalizeHostError(error).message}`,
    };
  }
  renderWorkspace();
}

async function saveProviderSettings(event) {
  event.preventDefault();
  const form = event.currentTarget;
  const providerId = state.selectedProviderId;
  setFormBusy(form, true);
  try {
    const previous = state.providerSettings[providerId];
    const credentialRequired = providerRequiresCredential(providerId);
    const apiKey = credentialRequired ? formValue(form, "apiKey") : "";
    if (apiKey) {
      await invokeHost("store_provider_secret", {
        reference: credentialReference(providerId),
        secret: apiKey,
      });
    }
    const enabled = form.elements.namedItem("enabled").checked;
    if (enabled && credentialRequired && !apiKey && !previous.credentialExists) {
      throw { code: "PROVIDER_CREDENTIAL_MISSING", message: "启用供应商前请先填写 API Key。" };
    }
    state.providerSettings[providerId] = {
      ...previous,
      enabled,
      defaultModel: formValue(form, "defaultModel"),
      qwenRegion: providerId === "qwen" ? formValue(form, "qwenRegion") : "china",
      qwenWorkspaceId: providerId === "qwen" ? formValue(form, "qwenWorkspaceId") : "",
      credentialExists: apiKey ? true : previous.credentialExists,
    };
    persistProviderSettings();
    await refreshProviderSecretStatus();
    setNotice(
      "success",
      credentialRequired
        ? `${PROVIDER_PRESETS[providerId].label} 设置已保存；明文凭据未返回 WebView。`
        : `${PROVIDER_PRESETS[providerId].label} 设置已保存；请求只会发往固定回环端点。`,
    );
  } catch (error) {
    setNotice("error", normalizeHostError(error).message);
  } finally {
    setFormBusy(form, false);
    renderWorkspace();
  }
}

async function deleteProviderCredential() {
  const providerId = state.selectedProviderId;
  if (!providerRequiresCredential(providerId)) return;
  try {
    await invokeHost("delete_provider_secret", { reference: credentialReference(providerId) });
    state.providerSettings[providerId] = {
      ...state.providerSettings[providerId],
      enabled: false,
      credentialExists: false,
    };
    persistProviderSettings();
    setNotice("success", `${PROVIDER_PRESETS[providerId].label} 凭据已从系统保险库删除。`);
  } catch (error) {
    setNotice("error", normalizeHostError(error).message);
  }
  renderWorkspace();
}

async function loadVersionHistory() {
  try {
    state.versionHistory = await invokeHost("get_version_history");
    renderWorkspace();
  } catch (error) {
    setNotice("error", normalizeHostError(error).message);
    state.versionsOpen = false;
    renderWorkspace();
  }
}

function versionDrawer() {
  const history = state.versionHistory;
  const drawer = element("aside", { className: "version-drawer" }, [
    element("div", { className: "drawer-heading" }, [
      element("div", {}, [element("span", { className: "eyebrow", text: "VERSION GRAPH" }), element("h2", { text: "版本历史" })]),
      button("×", "icon-button", toggleVersions, { title: "关闭" }),
    ]),
  ]);
  if (!history) {
    drawer.append(element("p", { className: "drawer-empty", text: "正在读取版本…" }));
    return drawer;
  }
  drawer.append(element("h3", { text: `检查点 · ${history.checkpoints.length}` }));
  if (!history.checkpoints.length) drawer.append(element("p", { className: "drawer-empty", text: "尚未建立检查点。" }));
  for (const checkpoint of history.checkpoints) {
    drawer.append(element("article", { className: "version-card" }, [
      element("strong", { text: formatDate(checkpoint.createdAt) }),
      element("code", { text: checkpoint.commitId.slice(0, 18) }),
      button("恢复到这里", "small-button", () => restoreCheckpoint(checkpoint.id)),
    ]));
  }
  drawer.append(element("h3", { text: `提交 · ${history.commits.length}` }));
  for (const commit of [...history.commits].reverse().slice(0, 50)) {
    drawer.append(element("article", { className: "commit-row" }, [
      element("span", { className: `commit-dot${commit.id === history.headCommitId ? " current" : ""}` }),
      element("div", {}, [
        element("strong", { text: commit.reason }),
        element("span", { text: formatDate(commit.createdAt) }),
      ]),
    ]));
  }
  return drawer;
}

function formatDate(value) {
  const date = new Date(value);
  return Number.isNaN(date.valueOf()) ? value : new Intl.DateTimeFormat("zh-CN", { dateStyle: "short", timeStyle: "short" }).format(date);
}

async function restoreCheckpoint(checkpointId) {
  if (!window.confirm("恢复会创建一个新的 Commit，当前历史不会被删除。继续吗？")) return;
  try {
    await flushAll();
    const response = await invokeHost("restore_checkpoint", {
      input: { schemaVersion: 1, checkpointId },
    });
    state.workspace = response.workspace;
    [state.archivedDocuments, state.summaryInvalidations] = await Promise.all([
      invokeHost("list_archived_documents"),
      invokeHost("list_summary_invalidations"),
    ]);
    state.session = {
      ...state.session,
      project: {
        ...state.session.project,
        headCommitId: response.workspace.headCommitId,
        revision: response.workspace.revision,
      },
    };
    state.selectedDocumentId = selectInitialDocument(response.workspace, state.selectedDocumentId);
    state.versionHistory = null;
    state.conflictDraft = null;
    setNotice("success", `已恢复 ${response.changedBlocks} 个 Block，并创建新的恢复提交。`);
    scheduleSummaryRefresh();
    await loadVersionHistory();
  } catch (error) {
    setNotice("error", normalizeHostError(error).message);
    renderWorkspace();
  }
}

async function closeProject() {
  try {
    clearTimeout(state.summaryRefreshTimer);
    state.summaryRefreshTimer = null;
    state.aiRunning?.controller.abort();
    await flushAll({ allowConflict: true });
    if (state.conflictDraft && !window.confirm("仍有冲突草稿未处理，确定关闭项目吗？")) return;
    state.session = await invokeHost("close_project");
    state.workspace = null;
    state.selectedDocumentId = null;
    state.versionHistory = null;
    state.versionsOpen = false;
    state.candidatesOpen = false;
    state.reviewCandidates = [];
    state.candidateBusy = null;
    state.providersOpen = false;
    state.stylesOpen = false;
    state.styleSamples = [];
    state.knowledgeItems = [];
    state.archivedDocuments = [];
    state.summaryInvalidations = [];
    clearTimeout(state.summaryRefreshTimer);
    state.summaryRefreshTimer = null;
    state.summaryRefreshBusy = false;
    state.summaryRefreshFailures = 0;
    state.recentProjectBusy = null;
    state.conflictDraft = null;
    state.aiRunning = null;
    state.aiContextPreview = null;
    state.aiReview = null;
    state.aiContextMenu = null;
    state.commandPaletteOpen = false;
    state.activeBlockId = null;
    state.aiSelection = null;
    setNotice(null, null);
    await loadRecentProjects();
    renderWelcome();
  } catch (error) {
    setNotice("error", normalizeHostError(error).message);
    renderWorkspace();
  }
}

window.addEventListener("beforeunload", (event) => {
  if (state.pendingText.size || state.savePromises.size) event.preventDefault();
});

document.addEventListener("keydown", (event) => {
  if (event.key === "Escape") {
    if (state.aiContextPreview) {
      event.preventDefault();
      cancelContextPreview();
    } else if (state.commandPaletteOpen || state.aiContextMenu) {
      event.preventDefault();
      closeCommandSurfaces();
    }
    return;
  }
  if ((event.ctrlKey || event.metaKey) && event.shiftKey && event.key.toLowerCase() === "p") {
    if (!state.workspace || state.aiContextPreview) return;
    event.preventDefault();
    openCommandPalette();
    return;
  }
  if (!event.altKey || event.ctrlKey || event.metaKey || event.shiftKey || !state.workspace) return;
  const activeTag = document.activeElement?.tagName;
  if (["INPUT", "TEXTAREA", "SELECT"].includes(activeTag)) return;
  const command = AI_OPERATION_COMMANDS.find((item) => item.key === event.key);
  if (!command || operationCommandsDisabled() || state.aiContextPreview) return;
  event.preventDefault();
  executeRegisteredCommand(command.type);
});

document.addEventListener("pointerdown", (event) => {
  if (
    !state.aiContextMenu
    || (event.target instanceof Element && event.target.closest(".ai-context-menu"))
  ) return;
  state.aiContextMenu = null;
  document.querySelector(".ai-context-menu")?.remove();
});

window.addEventListener("resize", () => {
  if (state.aiContextMenu) closeCommandSurfaces();
});

bootstrap();
