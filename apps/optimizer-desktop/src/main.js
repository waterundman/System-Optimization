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
  matchesDesktopRetryTarget,
  persistDesktopReviewDecision,
  planDesktopReviewDecisions,
  providerRequiresCredential,
  rejectDesktopReview,
  retryableOperationFailure,
  runDesktopOperation,
} from "./operation-client.js";
import { PRICING_TABLE, estimateFees } from "./insights-pricing.js";
import { t, setLocale } from "./i18n.js";
import { trapFocus, announceLive } from "./a11y.js";

const app = typeof document !== "undefined" ? document.querySelector("#app") : null;
const PROVIDER_SETTINGS_KEY = "optimizer.provider-settings.v1";
const SUMMARY_REFRESH_DEBOUNCE_MS = 800;
const TIMELINE_STATE_COLORS = Object.freeze({
  draft: "#9ca3af",
  compiling: "#3b82f6",
  preflight: "#06b6d4",
  queued: "#2563eb",
  streaming: "#8b5cf6",
  validating: "#f59e0b",
  review: "#eab308",
  accepted: "#10b981",
  rejected: "#ef4444",
  conflicted: "#f97316",
  failed: "#b91c1c",
  cancelled: "#6b7280",
});
const TIMELINE_STATE_LABELS = Object.freeze({
  draft: t("timeline.state.draft"),
  compiling: t("timeline.state.compiling"),
  preflight: t("timeline.state.preflight"),
  queued: t("timeline.state.queued"),
  streaming: t("timeline.state.streaming"),
  validating: t("timeline.state.validating"),
  review: t("timeline.state.review"),
  accepted: t("timeline.state.accepted"),
  rejected: t("timeline.state.rejected"),
  conflicted: t("timeline.state.conflicted"),
  failed: t("timeline.state.failed"),
  cancelled: t("timeline.state.cancelled"),
});
const AI_OPERATION_COMMANDS = Object.freeze([
  { type: "continue_scene", icon: "continue", label: t("ai.op.continue_scene.label"), description: t("ai.op.continue_scene.description"), shortcut: "Alt+1", key: "1" },
  { type: "polish", icon: "polish", label: t("ai.op.polish.label"), description: t("ai.op.polish.description"), shortcut: "Alt+2", key: "2" },
  { type: "compress", icon: "compress", label: t("ai.op.compress.label"), description: t("ai.op.compress.description"), shortcut: "Alt+3", key: "3" },
  { type: "expand", icon: "expand", label: t("ai.op.expand.label"), description: t("ai.op.expand.description"), shortcut: "Alt+4", key: "4" },
  { type: "critique", icon: "critique", label: t("ai.op.critique.label"), description: t("ai.op.critique.description"), shortcut: "Alt+5", key: "5" },
]);
const state = {
  session: null,
  workspace: null,
  selectedDocumentId: null,
  locale: "zh-CN",
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
  trustedModelEndpoints: [],
  compatibleDiscovery: null,
  versionHistory: null,
  reviewCandidates: [],
  candidateBusy: null,
  saveTimers: new Map(),
  pendingText: new Map(),
  savePromises: new Map(),
  saveStatus: t("common.saved"),
  notice: null,
  conflictDraft: null,
  activeBlockId: null,
  aiSelection: null,
  aiRunning: null,
  aiContextPreview: null,
  aiReview: null,
  aiRetry: null,
  aiContextMenu: null,
  commandPaletteOpen: false,
  insightsOpen: false,
  insightsData: null,
  insightsBusy: null,
  compareOpen: false,
  compareBusy: false,
  compareResult: null,
  compareSelection: {
    snapshotIdA: "",
    snapshotIdB: "",
    documentIdA: "",
    documentIdB: "",
  },
  timelineOpen: false,
  timelineEvents: null,
  timelineBusy: false,
  timelineSelected: null,
  showRevisionMetrics: false,
  backupWizardOpen: false,
  backupWizardStep: 1,
  backupWizardMode: null,
  backupWizardBusy: false,
  backupWizardResult: null,
  backupWizardForm: {
    includeEndpoints: true,
    includeRecent: true,
    outputPath: "",
    archivePath: "",
    targetDirectory: "",
    newProjectId: "",
    overwrite: false,
  },
  backupWizardManifest: null,
  backupWizardWarnings: [],
};

const SVG_NAMESPACE = "http:" + "//www.w3.org/2000/svg";

function element(tag, options = {}, children = [], doc = (typeof document !== "undefined" ? document : null)) {
  // v0.7.0 Stage 1 (D7): SVG namespace support. Tags prefixed with "svg:"
  // are created via createElementNS so the SVG overlay and its <path>
  // children sit in the SVG namespace and are addressable by SVG-aware
  // CSS and querySelector. The "svg:" prefix is stripped from the
  // qualified name passed to createElementNS so the resulting element's
  // tagName is the plain local name (e.g. "svg", "path", "g") rather than
  // "svg:svg" — this matches the behaviour of real browsers and keeps
  // querySelectorAll("path") working in linkedom-based unit tests. Plain
  // HTML tags still use createElement so the v0.6.0 baseline callers are
  // unaffected.
  const isSvg = tag.startsWith("svg:");
  const qualifiedName = isSvg ? tag.slice("svg:".length) : tag;
  const item = isSvg
    ? doc.createElementNS(SVG_NAMESPACE, qualifiedName)
    : doc.createElement(tag);
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

function button(label, className, onClick, options = {}, doc = (typeof document !== "undefined" ? document : null)) {
  const item = element("button", { className, text: label, type: "button", ...options }, [], doc);
  if (onClick) item.addEventListener("click", onClick);
  return item;
}

// SVG icon paths — 16x16 viewBox, 1.4 stroke, currentColor
const ICON_PATHS = Object.freeze({
  chapter: "M4 2h5l3 3v9H4z M9 2v3h3 M6 8h4 M6 10h3",
  text: "M3 4.5h10 M3 7.5h10 M3 10.5h7",
  plus: "M8 3v10 M3 8h10",
  up: "M8 3.5v9 M4.5 7L8 3.5L11.5 7",
  down: "M8 3.5v9 M4.5 9L8 12.5L11.5 9",
  edit: "M3 13l1-3 6-6 2 2-6 6z M9 4l2 2",
  close: "M4 4l8 8 M12 4l-8 8",
  continue: "M3 8h8 M8 5l3 3-3 3",
  polish: "M8 3L9.5 6.5L13 8L9.5 9.5L8 13L6.5 9.5L3 8L6.5 6.5Z",
  compress: "M2 8h4 M6 5l2 3-2 3 M14 8h-4 M10 5l-2 3 2 3",
  expand: "M8 3v4 M8 9v4 M5 6L8 3L11 6 M5 10L8 13L11 10",
  critique: "M3 3h10v7H7l-2 2v-2H3z M5.5 6h5 M5.5 8h3",
  download: "M8 2v6 M5.5 5L8 7.5 10.5 5 M3 13h10",
  upload: "M8 9V3 M5.5 6L8 3.5 10.5 6 M3 13h10",
  flag: "M4 2v12 M4 3h7l-1.5 2.5L11 8H4",
  indent: "M3 4h10 M3 8h6 M3 12h10 M9 6l2 2-2 2",
  outdent: "M3 4h10 M3 8h6 M3 12h10 M9 6l-2 2 2 2",
  restore: "M4 8a4 4 0 1 0 1.5-3 M4 4.5v2h2",
  compare: "M2 4h5v8H2z M9 4h5v8H9z",
  chart: "M3 13h10 M5 13V9 M8 13V5.5 M11 13V7.5",
  layers: "M8 3l5 3-5 3-5-3z M8 9l5 3-5 3-5-3",
  history: "M8 4a4 4 0 1 0 0.01 0z M8 6v2l1.5 1",
  timeline: "M4 3v10 M4 4h6 M4 8h4 M4 12h6",
  book: "M3 4h4.5v9H3z M8.5 4H13v9H8.5 M8 4v9",
  settings: "M8 5.5a2.5 2.5 0 1 0 0 5 2.5 2.5 0 0 0 0-5z M8 2v1.5 M8 12.5V14 M2 8h1.5 M12.5 8H14",
  backup: "M3 5l2-2h6l2 2v8H3z M8 8v3 M6 10l2 2 2-2",
});

function svgIcon(name, size = 14) {
  const d = ICON_PATHS[name];
  if (!d) return null;
  return element("svg:svg", {
    attrs: {
      viewBox: "0 0 16 16",
      width: String(size),
      height: String(size),
      fill: "none",
      stroke: "currentColor",
      "stroke-width": "1.4",
      "stroke-linecap": "round",
      "stroke-linejoin": "round",
      "aria-hidden": "true",
    },
  }, [element("svg:path", { attrs: { d } })]);
}

function iconOnlyButton(iconName, className, onClick, options = {}, doc = (typeof document !== "undefined" ? document : null)) {
  const item = element("button", { className, type: "button", ...options }, [], doc);
  const icon = svgIcon(iconName, 12);
  if (icon) item.append(icon);
  if (onClick) item.addEventListener("click", onClick);
  return item;
}

function iconTextButton(iconName, label, className, onClick, options = {}, doc = (typeof document !== "undefined" ? document : null)) {
  const item = element("button", { className, type: "button", ...options }, [], doc);
  const icon = svgIcon(iconName, 13);
  if (icon) item.append(icon);
  item.append(label);
  if (onClick) item.addEventListener("click", onClick);
  return item;
}

// v0.8.0 Stage 3 (a11y): install focus trap + Escape-to-close on a drawer
// container. `closeState` is called on Escape to flip the relevant state flag
// (e.g. state.timelineOpen = false). The trapFocus release function restores
// focus to the opener element. renderWorkspace is deferred via queueMicrotask
// so the keydown event finishes dispatching before the DOM is replaced.
function attachDrawerKeyboard(drawer, docFactory, closeState) {
  const release = trapFocus(drawer, { document: docFactory });
  drawer.addEventListener("keydown", (event) => {
    if (event.key !== "Escape") return;
    event.preventDefault();
    closeState();
    release();
    queueMicrotask(() => {
      try { renderWorkspace(); } catch { /* host unavailable in test env */ }
    });
  });
  return drawer;
}

async function invokeHost(command, args = {}) {
  const invoke = window.__TAURI__?.core?.invoke;
  if (typeof invoke !== "function") {
    throw { code: "HOST_UNAVAILABLE", message: t("error.hostUnavailable") };
  }
  return invoke(command, args);
}

// v0.8.0 无边框窗口：把最小化/最大化/关闭融进前端自定义标题栏。
// 依赖 Tauri 的 core:window 权限（start-dragging / minimize / toggle-maximize / close）。
function setupWindowControls() {
  const appWindow = window.__TAURI__?.window?.getCurrentWindow?.();
  if (!appWindow) return;
  const labels = {
    minimize: t("window.minimize"),
    maximize: t("window.maximize"),
    close: t("window.close"),
  };
  const applyAction = (action) => {
    if (action === "minimize") {
      appWindow.minimize();
    } else if (action === "maximize") {
      appWindow.toggleMaximize();
    } else if (action === "close") {
      appWindow.close();
    }
  };
  document.querySelectorAll("[data-win]").forEach((button) => {
    const action = button.getAttribute("data-win");
    button.removeAttribute("aria-hidden");
    button.setAttribute("aria-label", labels[action] || action);
    button.title = labels[action] || action;
    button.addEventListener("click", () => applyAction(action));
  });
}

function setNotice(kind, message, actionLabel, action) {
  state.notice = message ? { kind, message, actionLabel, action } : null;
}

function noticeView() {
  if (!state.notice) return null;
  return element("div", {
    className: `notice notice-${state.notice.kind}`,
    attrs: { role: "status" },
  }, [
    element("span", { text: state.notice.message }),
    state.notice.actionLabel && typeof state.notice.action === "function"
      ? button(state.notice.actionLabel, "small-button notice-action", state.notice.action, {
          disabled: Boolean(state.aiRunning || state.aiReview),
        })
      : null,
  ]);
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
        trustedEndpointId: typeof value.trustedEndpointId === "string"
          ? value.trustedEndpointId
          : "",
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
      trustedEndpointId: value.trustedEndpointId || "",
    },
  ]));
  localStorage.setItem(PROVIDER_SETTINGS_KEY, JSON.stringify(safe));
}

async function refreshProviderSecretStatus() {
  const entries = await Promise.all(Object.keys(PROVIDER_PRESETS).map(async (providerId) => {
    if (!providerRequiresCredential(providerId)) return [providerId, false];
    const endpoint = providerId === "openai_compatible"
      ? state.providerSettings[providerId].trustedEndpoint
      : null;
    if (providerId === "openai_compatible" && !endpoint) return [providerId, false];
    try {
      const status = await invokeHost("has_provider_secret", {
        reference: credentialReference(providerId, endpoint),
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

async function refreshTrustedModelEndpoints() {
  const endpoints = await invokeHost("list_trusted_model_endpoints");
  state.trustedModelEndpoints = Array.isArray(endpoints)
    ? endpoints.filter((endpoint) => endpoint
      && /^endpoint-[a-f0-9]{32}$/.test(endpoint.id)
      && endpoint.revision === 1
      && typeof endpoint.baseUrl === "string"
      && typeof endpoint.credentialRef === "string")
    : [];
  const settings = state.providerSettings.openai_compatible;
  const trustedEndpoint = state.trustedModelEndpoints.find(
    (endpoint) => endpoint.id === settings.trustedEndpointId,
  ) ?? null;
  state.providerSettings.openai_compatible = {
    ...settings,
    trustedEndpoint,
    trustedEndpointId: trustedEndpoint?.id ?? "",
    enabled: trustedEndpoint ? settings.enabled : false,
  };
}

async function bootstrap() {
  try {
    setupWindowControls();
    // v0.8.0 Stage 2 leftover: load the persisted user locale from the host
    // before any UI renders so the very first paint matches the user's
    // saved preference. setLocale is imported from ./i18n.js but was
    // previously never invoked at startup, leaving state.locale stuck on
    // the hardcoded "zh-CN" default.
    await applyPersistedLocale();
    await refreshTrustedModelEndpoints();
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

async function applyPersistedLocale() {
  try {
    const response = await invokeHost("get_user_locale");
    if (response && typeof response.locale === "string" && response.locale) {
      state.locale = response.locale;
      setLocale(response.locale);
    }
  } catch {
    // Host unavailable (e.g. unit tests / preview) — keep the default locale.
  }
}

async function persistLocale(locale) {
  if (typeof locale !== "string" || !locale) return false;
  state.locale = locale;
  setLocale(locale);
  try {
    const response = await invokeHost("set_user_locale", {
      input: { schemaVersion: 1, locale },
    });
    if (response && typeof response.locale === "string") {
      state.locale = response.locale;
      setLocale(response.locale);
      announceLive(t("a11y.localeSwitched"));
      return true;
    }
  } catch (error) {
    setNotice("error", normalizeHostError(error).message);
  }
  return false;
}

function renderWelcome() {
  disposeVirtualControllers();
  const createForm = element("form", { className: "create-form" }, [
    labeledInput(t("welcome.create.titleLabel"), "title", t("welcome.create.titlePlaceholder"), true),
    directoryField(t("welcome.create.saveTo"), "parentDirectory", t("welcome.create.saveToPlaceholder")),
    labeledInput(t("welcome.create.folderLabel"), "folderName", t("welcome.create.folderPlaceholder"), true),
    element("p", { className: "form-hint", text: t("welcome.create.hint") }),
    element("button", { className: "primary-button create-form-submit", text: t("welcome.create.submit"), type: "submit" }),
  ]);
  createForm.addEventListener("submit", createProject);

  const openForm = element("form", { className: "open-form" }, [
    labeledInput(t("welcome.open.pathLabel"), "projectDirectory", t("welcome.open.pathPlaceholder"), true),
    element("p", { className: "form-hint", text: t("welcome.open.hint") }),
    element("button", { className: "secondary-button open-form-submit", text: t("welcome.open.submit"), type: "submit" }),
  ]);
  openForm.addEventListener("submit", openProject);

  app.replaceChildren(element("div", { className: "welcome-container" }, [
    element("div", { className: "welcome-header" }, [
      element("div", { className: "welcome-brand" }, [
        element("div", { className: "brand-mark brand-mark-large", text: t("common.brandMark") }),
      ]),
    ]),
    element("div", { className: "welcome-content" }, [
      element("div", { className: "welcome-main" }, [
        element("div", { className: "welcome-block welcome-block-create" }, [
          element("div", { className: "welcome-block-header" }, [
            element("h2", { text: t("welcome.create.title") }),
          ]),
          createForm,
        ]),
      ]),
      element("div", { className: "welcome-sidebar" }, [
        element("div", { className: "welcome-block welcome-block-open" }, [
          element("div", { className: "welcome-block-header" }, [
            element("h2", { text: t("welcome.open.title") }),
          ]),
          openForm,
        ]),
        recentProjectsView(),
      ]),
    ]),
    noticeView(),
  ]));
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
      element("h2", { text: t("recent.title"), attrs: { id: "recent-projects-title" } }),
      element("span", { text: t("recent.count", { count: state.recentProjects.length }) }),
    ]),
    ...state.recentProjects.map(recentProjectRow),
  ]);
}

function recentProjectRow(project) {
  const busy = state.recentProjectBusy === project.projectId;
  const open = button("", "recent-project-open", () => openRecentProject(project), {
    disabled: busy || !project.available,
    attrs: { "aria-label": t("recent.openLabel", { title: project.title }) },
  });
  open.append(
    element("span", { className: "recent-project-mark", text: t("common.brandMark") }),
    element("span", { className: "recent-project-copy" }, [
      element("strong", { text: project.title }),
      element("span", { text: project.available ? project.directory : t("recent.unavailable") }),
    ]),
    element("span", { className: "recent-project-time", text: formatRecentTime(project.lastOpenedAt) }),
  );
  return element("div", { className: `recent-project${project.available ? "" : " recent-project-missing"}` }, [
    open,
    button(t("recent.remove"), "recent-project-remove", () => removeRecentProject(project), {
      disabled: busy,
      attrs: { "aria-label": t("recent.removeLabel", { title: project.title }) },
    }),
  ]);
}

function formatRecentTime(value) {
  const date = new Date(value);
  if (Number.isNaN(date.getTime())) return t("recent.lastOpened");
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
    setNotice("success", t("recent.removed", { title: project.title }));
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

// 目录选择字段 — 文本输入 + 「浏览…」按钮，点击后通过系统目录选择器选取路径。
function directoryField(label, name, placeholder) {
  const input = element("input", { name, placeholder, attrs: { autocomplete: "off" } });
  const browse = button(t("welcome.create.browse"), "small-button", (event) => {
    event.preventDefault();
    const picker = element("input", {
      type: "file",
      attrs: { "aria-label": label, webkitdirectory: "", directory: "" },
    });
    picker.hidden = true;
    picker.addEventListener("change", () => {
      const file = picker.files?.[0];
      picker.remove();
      if (file?.path) input.value = file.path;
    }, { once: true });
    (document.body || document).append(picker);
    picker.click();
  });
  return element("label", { className: "field" }, [
    element("span", { text: label }),
    element("div", { className: "field-row" }, [input, browse]),
  ]);
}

function formValue(form, name) {
  return String(new FormData(form).get(name) ?? "").trim();
}

async function createProject(event) {
  event.preventDefault();
  const form = event.currentTarget;
  const input = {
    schemaVersion: 1,
    parentDirectory: formValue(form, "parentDirectory"),
    folderName: formValue(form, "folderName"),
    title: formValue(form, "title"),
    language: state.locale || "zh-CN",
  };
  setFormBusy(form, true);
  try {
    state.session = await invokeHost("create_project", { input });
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
  const input = {
    schemaVersion: 1,
    projectDirectory: formValue(form, "projectDirectory"),
  };
  setFormBusy(form, true);
  try {
    state.session = await invokeHost("open_project", { input });
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
  await refreshTrustedModelEndpoints();
  await refreshProviderSecretStatus();
  renderWorkspace();
  scheduleSummaryRefresh();
}

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
    attrs: { role: "menu", "aria-label": t("ai.menu.title") },
  }, [
    element("div", { className: "context-menu-heading", text: t("ai.menu.title") }),
    ...AI_OPERATION_COMMANDS.map((command) => commandMenuButton(command)),
    element("div", { className: "context-menu-separator", attrs: { role: "separator" } }),
    commandMenuButton({
      type: "pin_style",
      label: t("ai.menu.pinStyle"),
      description: t("ai.menu.saveSelection"),
      shortcut: "",
    }),
    element("div", { className: "context-menu-hint", text: t("ai.menu.hint") }),
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
  // v0.8.0 Stage 3 (a11y): the palette is a real listbox so screen readers
  // announce the active option as the user arrows through it. The query
  // input owns the keyboard: ArrowUp/Down move the active option, Enter
  // executes it, Escape closes. aria-activedescendant on the listbox points
  // at the currently active option's id so AT speaks it without moving DOM
  // focus away from the input.
  let activeIndex = 0;
  const commands = [
    ...AI_OPERATION_COMMANDS,
    {
      type: "pin_style",
      label: t("ai.menu.pinStyle"),
      description: t("ai.menu.saveToLibrary"),
      shortcut: "",
    },
  ];
  const commandRows = commands.map((command, index) => {
    const row = button("", "command-palette-item", () => executeRegisteredCommand(command.type), {
      disabled: operationCommandsDisabled(),
      attrs: {
        "data-command-search": `${command.label} ${command.description} ${command.type}`.toLowerCase(),
        role: "option",
        id: `command-palette-option-${index}`,
        "aria-selected": index === 0 ? "true" : "false",
      },
    });
    row.append(element("span", { className: "command-palette-copy" }, [
      element("strong", { text: command.label }),
      element("span", { text: command.description }),
    ]));
    if (command.shortcut) row.append(element("kbd", { text: command.shortcut }));
    return row;
  });
  const list = element("div", {
    className: "command-palette-list",
    attrs: {
      id: "command-palette-list",
      role: "listbox",
      "aria-labelledby": "command-palette-title",
      "aria-activedescendant": commandRows[0] ? commandRows[0].id : "",
    },
  }, commandRows);
  const query = element("input", {
    type: "search",
    placeholder: t("ai.palette.searchPlaceholder"),
    attrs: {
      id: "command-palette-query",
      "aria-label": t("ai.palette.searchLabel"),
      "aria-controls": "command-palette-list",
      "aria-autocomplete": "list",
      "aria-expanded": "true",
      "aria-activedescendant": commandRows[0] ? commandRows[0].id : "",
      autocomplete: "off",
    },
  });
  const updateActive = (nextIndex) => {
    if (nextIndex < 0 || nextIndex >= commandRows.length) return;
    activeIndex = nextIndex;
    for (let i = 0; i < commandRows.length; i++) {
      commandRows[i].setAttribute("aria-selected", i === activeIndex ? "true" : "false");
    }
    const activeId = commandRows[activeIndex].id;
    list.setAttribute("aria-activedescendant", activeId);
    query.setAttribute("aria-activedescendant", activeId);
  };
  const visibleIndices = () => commandRows
    .map((row, i) => ({ row, i }))
    .filter(({ row }) => !row.hidden && !row.disabled)
    .map(({ i }) => i);
  query.addEventListener("input", () => {
    const needle = query.value.trim().toLowerCase();
    for (const row of commandRows) {
      row.hidden = !row.dataset.commandSearch.includes(needle);
    }
    const firstVisible = visibleIndices()[0];
    if (firstVisible !== undefined) updateActive(firstVisible);
  });
  query.addEventListener("keydown", (event) => {
    if (event.key === "Escape") {
      event.preventDefault();
      closeCommandSurfaces();
    } else if (event.key === "Enter") {
      event.preventDefault();
      const active = commandRows[activeIndex];
      if (active && !active.hidden && !active.disabled) active.click();
    } else if (event.key === "ArrowDown") {
      event.preventDefault();
      const visible = visibleIndices();
      if (!visible.length) return;
      const currentPos = visible.indexOf(activeIndex);
      const nextPos = currentPos < 0 ? 0 : (currentPos + 1) % visible.length;
      updateActive(visible[nextPos]);
    } else if (event.key === "ArrowUp") {
      event.preventDefault();
      const visible = visibleIndices();
      if (!visible.length) return;
      const currentPos = visible.indexOf(activeIndex);
      const prevPos = currentPos < 0
        ? 0
        : (currentPos - 1 + visible.length) % visible.length;
      updateActive(visible[prevPos]);
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
          element("h2", { text: t("ai.palette.title"), attrs: { id: "command-palette-title" } }),
        ]),
        element("kbd", { text: "Esc" }),
      ]),
      query,
      operationCommandsDisabled()
        ? element("p", { className: "command-palette-warning", text: t("ai.palette.warning") })
        : null,
      list,
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

async function runAiOperation(operationType, retryTarget = null) {
  if (state.aiRunning || state.aiReview?.kind === "patch_proposal") return;
  const providerSettings = state.providerSettings[state.selectedProviderId];
  const credentialRequired = providerRequiresCredential(state.selectedProviderId);
  if (!providerSettings.enabled || (credentialRequired && !providerSettings.credentialExists)) {
    state.providersOpen = true;
    state.versionsOpen = false;
    state.stylesOpen = false;
    state.candidatesOpen = false;
    state.timelineOpen = false;
    setNotice(
      "warning",
      credentialRequired
        ? t("ai.op.providerDisabled", { label: PROVIDER_PRESETS[state.selectedProviderId].label })
        : t("ai.op.providerDisabledNoKey", { label: PROVIDER_PRESETS[state.selectedProviderId].label }),
    );
    renderWorkspace();
    return;
  }
  let attemptedTarget = retryTarget;
  state.aiRetry = null;
  try {
    await flushAll();
    const documentBlocks = blocksForDocument(state.workspace, state.selectedDocumentId);
    const block = retryTarget
      ? state.workspace.blocks.find((candidate) => candidate.id === retryTarget.blockId)
      : state.workspace.blocks.find((candidate) => candidate.id === state.activeBlockId)
        ?? documentBlocks.find(isEditableBlock);
    if (!block || !isEditableBlock(block)) {
      throw { code: "TARGET_INVALID", message: t("error.targetInvalid") };
    }
    let from;
    let to;
    if (retryTarget) {
      if (!matchesDesktopRetryTarget(block, retryTarget)) {
        throw { code: "TARGET_STALE", message: t("error.targetStale") };
      }
      from = retryTarget.from;
      to = retryTarget.to;
    } else {
      const selection = state.aiSelection?.blockId === block.id ? state.aiSelection : null;
      from = Math.min(selection?.from ?? 0, block.plainText.length);
      to = Math.min(selection?.to ?? block.plainText.length, block.plainText.length);
      if (operationType === "continue_scene") {
        const caret = selection ? selection.to : block.plainText.length;
        from = caret;
        to = caret;
      } else if (from === to) {
        from = 0;
        to = block.plainText.length;
      }
    }
    attemptedTarget = {
      blockId: block.id,
      baseRevision: block.revision,
      baseHash: block.contentHash,
      from,
      to,
    };
    const Channel = window.__TAURI__?.core?.Channel;
    if (typeof Channel !== "function") {
      throw { code: "HOST_UNAVAILABLE", message: t("error.streamUnsupported") };
    }
    const controller = new AbortController();
    state.aiRunning = {
      controller,
      label: t("ai.op.compilingContext"),
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
      state.aiRetry = null;
      setNotice("success", t("ai.op.generated", { count: execution.result.proposal.hunks.length }));
    } else {
      state.aiReview = {
        kind: "findings",
        intent: execution.intent,
        result: execution.result,
      };
      state.aiRetry = null;
      setNotice("success", t("ai.op.critiqueDone", { count: execution.result.findings.length }));
    }
  } catch (error) {
    const normalized = normalizeHostError(error);
    const retry = attemptedTarget ? retryableOperationFailure(error) : null;
    if (retry && attemptedTarget) {
      state.aiRetry = { operationType, target: attemptedTarget };
      const attempted = Math.max(1, retry.attempts);
      setNotice(
        "warning",
        t("ai.op.retryFailed", { attempts: attempted, message: normalized.message }),
        t("ai.op.retry"),
        retryLastAiOperation,
      );
    } else {
      state.aiRetry = null;
      setNotice(normalized.code === "OPERATION_CANCELLED" ? "warning" : "error", normalized.message);
    }
  } finally {
    state.aiContextPreview = null;
    state.aiRunning = null;
    renderWorkspace();
  }
}

function retryLastAiOperation() {
  const retry = state.aiRetry;
  if (!retry || state.aiRunning || state.aiReview) return;
  state.aiRetry = null;
  setNotice(null, null);
  renderWorkspace();
  void runAiOperation(retry.operationType, retry.target);
}

function confirmCompiledContext(packet) {
  return new Promise((resolve, reject) => {
    state.aiContextPreview = { packet, resolve, reject };
    if (state.aiRunning) state.aiRunning.label = t("ai.op.waitingConfirm");
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
          element("h2", { text: t("ai.context.title"), attrs: { id: "context-title" } }),
        ]),
        element("span", {
          className: "count-pill",
          text: t("ai.context.tokens", { count: packet.budget.estimatedInput.toLocaleString("zh-CN") }),
        }),
      ]),
      element("p", {
        className: "context-explainer",
        text: t("ai.context.description"),
      }),
      element("div", { className: "context-items" }, packet.items.map((item) => element("article", {
        className: "context-item",
      }, [
        element("div", { className: "context-item-heading" }, [
          element("strong", { text: item.tier }),
          element("code", { text: item.sourceRef }),
          item.mandatory ? element("span", { className: "mandatory-pill", text: t("common.mandatory") }) : null,
        ]),
        element("pre", { text: item.content || t("common.emptySelection") }),
        element("span", { className: "context-reason", text: item.reasonCodes.join(" · ") }),
      ]))),
      packet.exclusions.length
        ? element("details", { className: "context-exclusions" }, [
            element("summary", { text: t("ai.context.exclusions", { count: packet.exclusions.length }) }),
            ...packet.exclusions.map((item) => element("code", {
              text: `${item.sourceRef} · ${item.reason}`,
            })),
          ])
        : null,
      element("div", { className: "modal-actions" }, [
        button(t("ai.context.cancel"), "quiet-button", cancelContextPreview),
        button(t("ai.context.confirm"), "primary-button context-confirm", approveContextPreview),
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
  updateAiStatus(t("ai.op.modelGenerating"));
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
      compiling: t("ai.op.status.compiling"),
      preflight: t("ai.op.status.preflight"),
      queued: t("ai.op.status.queued"),
      streaming: t("ai.op.status.streaming"),
      validating: t("ai.op.status.validating"),
      review: t("ai.op.status.review"),
    };
    state.aiRunning.label = labels[event.transition.to] ?? state.aiRunning.label;
  } else if (event.type === "model_text_delta") {
    state.aiRunning.received += event.text.length;
    state.aiRunning.label = t("ai.op.modelGeneratingChars", { count: state.aiRunning.received.toLocaleString("zh-CN") });
  } else if (event.type === "model_reasoning_delta") {
    state.aiRunning.label = t("ai.op.modelReasoning");
  } else if (event.type === "model_retry") {
    state.aiRunning.received = 0;
    state.aiRunning.label = t("ai.op.retryPending", { delay: event.delayMs.toLocaleString("zh-CN"), attempt: event.nextAttempt });
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
  updateAiStatus(t("ai.op.canceling"));
}

function aiReviewView() {
  const review = state.aiReview;
  if (!review) return null;
  if (review.kind === "findings") {
    return element("section", { className: "ai-review-card findings-card" }, [
      element("div", { className: "review-heading" }, [
        element("div", {}, [
          element("span", { className: "eyebrow", text: "AI FINDINGS" }),
          element("h2", { text: t("ai.critique.title") }),
        ]),
        button(t("common.close"), "quiet-button", () => {
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
        element("h2", { text: t("review.title") }),
      ]),
      element("div", { className: "review-heading-actions" }, [
        element("span", { className: "count-pill", text: t("review.count", { accepted, rejected }) }),
        element("div", { className: "review-batch-actions", attrs: { "aria-label": t("review.batchLabel") } }, [
          button(t("review.acceptAll"), "small-button", () => decideAllCurrentHunks("accepted"), {
            disabled: review.busy || accepted === total,
          }),
          button(t("review.rejectAll"), "quiet-button", () => decideAllCurrentHunks("rejected"), {
            disabled: review.busy || rejected === total,
          }),
        ]),
      ]),
    ]),
    proposal.summary ? element("p", { className: "review-summary", text: proposal.summary }) : null,
    review.candidateBranch
      ? element("div", { className: "review-branch-notice" }, [
          element("span", { text: t("review.branchSaved") }),
          element("code", { text: review.candidateBranch.branchName }),
        ])
      : null,
    ...proposal.hunks.map((hunk, index) => hunkReviewView(review, hunk, index)),
    element("div", { className: "review-footer" }, [
      element("div", { className: "review-footer-group" }, [
        button(t("review.defer"), "quiet-button", deferCurrentReview, { disabled: review.busy }),
        button(t("review.reject"), "quiet-button", rejectCurrentReview, { disabled: review.busy }),
      ]),
      element("div", { className: "review-footer-group" }, [
        button(
          review.candidateBranch ? t("review.branchSavedShort") : t("review.saveBranch"),
          "secondary-button",
          createCurrentCandidateBranch,
          {
            disabled: review.busy
              || review.session.status !== "ready"
              || accepted === 0
              || Boolean(review.candidateBranch),
            title: accepted === 0
              ? t("review.branchHintDisabled")
              : t("review.branchHintEnabled"),
          },
        ),
        button(accepted ? t("review.applyAccepted") : t("review.completeNoChange"), "primary-button review-apply", applyCurrentReview, {
          disabled: review.busy || review.session.status !== "ready",
          title: review.session.status === "ready" ? t("review.applyTitle") : t("review.applyDisabled"),
        }),
      ]),
    ]),
  ]);
}

function hunkReviewView(review, hunk, index) {
  const decision = review.session.decisions[hunk.id];
  // v0.8.0 Stage 3 (a11y): each hunk is a focusable group so keyboard users
  // can Tab between hunks without descending into the diff text. Enter
  // accepts the current hunk, Shift+Enter rejects it. Tab/Shift+Tab wrap
  // between the first and last hunk so focus never escapes the review card.
  const article = element("article", {
    className: `review-hunk decision-${decision}`,
    attrs: {
      role: "group",
      "aria-label": t("a11y.hunkGroup", { index: index + 1 }),
      tabindex: "0",
    },
  }, [
    element("div", { className: "hunk-heading" }, [
      element("strong", { text: t("review.hunkIndex", { index: index + 1 }) }),
      element("span", { text: hunk.granularity }),
    ]),
    element("div", { className: "hunk-diff" }, [
      element("del", { text: hunk.original || "∅" }),
      element("ins", { text: hunk.replacement || "∅" }),
    ]),
    element("div", { className: "hunk-actions" }, [
      button(decision === "accepted" ? t("review.accepted") : t("review.accept"), "small-button", () => decideCurrentHunk(hunk.id, "accepted"), {
        disabled: review.busy || decision === "accepted",
      }),
      button(decision === "rejected" ? t("review.rejected") : t("review.reject"), "quiet-button", () => decideCurrentHunk(hunk.id, "rejected"), {
        disabled: review.busy || decision === "rejected",
      }),
    ]),
  ]);
  article.addEventListener("keydown", (event) => {
    // Only intercept when the hunk article itself is focused (not when a
    // child button holds focus — buttons have their own native Enter/Space
    // activation and their own place in the tab order).
    if (event.target !== article) return;
    if (event.key === "Enter") {
      event.preventDefault();
      if (event.shiftKey) {
        const rejectBtn = article.querySelector(".hunk-actions button.quiet-button");
        if (rejectBtn && !rejectBtn.disabled) rejectBtn.click();
      } else {
        const acceptBtn = article.querySelector(".hunk-actions button.small-button");
        if (acceptBtn && !acceptBtn.disabled) acceptBtn.click();
      }
    } else if (event.key === "Tab") {
      const parent = article.parentElement;
      if (!parent) return;
      const hunks = Array.from(parent.querySelectorAll(".review-hunk"));
      const currentIdx = hunks.indexOf(article);
      if (currentIdx < 0) return;
      if (event.shiftKey) {
        if (currentIdx === 0) {
          event.preventDefault();
          hunks[hunks.length - 1].focus();
        }
      } else if (currentIdx === hunks.length - 1) {
        event.preventDefault();
        hunks[0].focus();
      }
    }
  });
  return article;
}

// v0.8.0 Stage 3 (a11y): announce the current patch review state through
// the aria-live region so screen reader users hear decision updates without
// having to navigate back to the count pill. Reads from state.aiReview so
// callers only need to invoke it after mutating the review session.
function announceReviewState() {
  const review = state.aiReview;
  if (!review || review.kind !== "patch_proposal") return;
  const proposal = review.result.proposal;
  if (!proposal || !Array.isArray(proposal.hunks)) return;
  const accepted = Object.values(review.session.decisions).filter((v) => v === "accepted").length;
  const rejected = Object.values(review.session.decisions).filter((v) => v === "rejected").length;
  const total = proposal.hunks.length;
  announceLive(t("a11y.reviewSummary", { accepted, rejected, total }));
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
    announceReviewState();
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
    announceReviewState();
    setNotice(
      "success",
      decision === "accepted"
        ? t("review.allAccepted", { count: review.result.proposal.hunks.length })
        : t("review.allRejected", { count: review.result.proposal.hunks.length }),
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
    setNotice("success", t("review.deferred"));
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
  const suggested = t("review.branchSuggested", { date: new Intl.DateTimeFormat("zh-CN", {
    month: "2-digit",
    day: "2-digit",
    hour: "2-digit",
    minute: "2-digit",
  }).format(new Date()) });
  const branchName = window.prompt(t("review.branchPrompt"), suggested)?.trim();
  if (!branchName) return;
  if (branchName.length > 120) {
    setNotice("error", t("error.branchNameTooLong"));
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
      t("review.branchCreated", { name: response.branch.branchName }),
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
    setNotice("success", t("review.rejectedNotice"));
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
      setNotice("success", t("review.allRejectedNotice"));
      renderWorkspace();
      return;
    }
    if (compilation.status !== "ready_to_apply") {
      throw { code: "CONFLICT", message: t("error.proposalTargetChanged") };
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
    setNotice("success", t("review.applied", { count: response.acceptedHunks }));
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
  if (state.summaryRefreshBusy) return t("summary.updating", { count: state.summaryInvalidations.length });
  return state.summaryInvalidations.length
    ? t("summary.pending", { count: state.summaryInvalidations.length })
    : t("summary.ready");
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
    updateSaveStatus(t("common.saved"));
    return;
  }
  state.pendingText.set(blockId, plainText);
  updateSaveStatus(t("common.editing"));
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
  updateSaveStatus(t("common.saving"));
  const promise = invokeHost("save_block", { input: buildSaveBlockRequest(block, plainText) })
    .then(async (response) => {
      state.workspace = applySaveResponse(state.workspace, response);
      state.summaryInvalidations = await invokeHost("list_summary_invalidations");
      state.session = {
        ...state.session,
        project: { ...state.session.project, headCommitId: response.headCommitId, revision: response.projectRevision },
      };
      updateSaveStatus(t("common.saved"));
      scheduleSummaryRefresh();
    })
    .catch(async (error) => {
      const normalized = normalizeHostError(error);
      if (normalized.code === "NO_CHANGES") {
        updateSaveStatus(t("common.saved"));
        return;
      }
      if (normalized.code === "CONFLICT") {
        state.conflictDraft = { blockId, plainText };
        setNotice("warning", t("save.detectedConflict"));
        await loadWorkspace();
        updateSaveStatus(t("common.conflictState"));
        return;
      }
      state.pendingText.set(blockId, plainText);
      setNotice("error", normalized.message);
      updateSaveStatus(t("common.saveFailed"));
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
    throw { code: "CONFLICT", message: t("error.conflictDraftPending") };
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
    setNotice("success", t("checkpoint.created", { id: checkpoint.id.slice(0, 20) }));
    state.versionHistory = null;
    state.compareResult = null;
    if (state.versionsOpen || state.compareOpen) await loadVersionHistory();
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
    state.insightsOpen = false;
    state.compareOpen = false;
    state.timelineOpen = false;
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
        element("h2", { text: t("candidate.center") }),
      ]),
      iconOnlyButton("close", "icon-button", toggleCandidates, { title: t("candidate.close") }),
    ]),
    element("p", {
      className: "drawer-intro",
      text: t("candidate.description"),
    }),
    element("h3", { text: t("candidate.pending", { count: active.length }) }),
  ]);
  if (!active.length) {
    drawer.append(element("p", { className: "drawer-empty", text: t("candidate.empty") }));
  } else {
    drawer.append(...active.map(reviewCandidateCard));
  }
  if (history.length) {
    drawer.append(element("h3", { text: t("candidate.done", { count: history.length }) }));
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
      element("span", { text: t("candidate.hunkCount", { count: candidate.hunkCount }) }),
    ]),
    element("strong", { text: candidate.summary || (document ? document.title : t("candidate.defaultSummary")) }),
    element("span", { text: candidate.providerId + " · " + candidate.model }),
    element("span", { text: formatDate(candidate.updatedAt) }),
    candidate.candidateBranch
      ? element("div", { className: "candidate-branch-badge" }, [
          element("span", { text: t("candidate.branch") }),
          element("code", { text: candidate.candidateBranch.branchName }),
        ])
      : null,
  ]);
  if (canResume) {
    card.append(button(
      isCurrent ? t("candidate.reviewing") : busy ? t("candidate.restoring") : t("candidate.continue"),
      "small-button candidate-resume",
      () => openReviewCandidate(candidate.proposalId),
      { disabled: isCurrent || busy || Boolean(state.aiRunning || state.aiReview) },
    ));
  } else if (candidate.status === "conflicted") {
    card.append(element("p", {
      className: "candidate-note",
      text: t("candidate.staleTarget"),
    }));
  }
  return card;
}

function reviewCandidateStatusLabel(status) {
  return {
    review: t("candidate.status.review"),
    ready: t("candidate.status.ready"),
    conflicted: t("candidate.status.conflicted"),
    applied: t("candidate.status.applied"),
    rejected: t("candidate.status.rejected"),
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
      throw { code: "REVIEW_FINALIZED", message: t("error.reviewFinalized") };
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
    setNotice("success", t("review.restored"));
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
    state.insightsOpen = false;
    state.compareOpen = false;
    state.timelineOpen = false;
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
    state.insightsOpen = false;
    state.compareOpen = false;
    state.timelineOpen = false;
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
    state.insightsOpen = false;
    state.compareOpen = false;
    state.timelineOpen = false;
  }
  renderWorkspace();
}

async function toggleInsights() {
  state.insightsOpen = !state.insightsOpen;
  if (state.insightsOpen) {
    state.providersOpen = false;
    state.stylesOpen = false;
    state.candidatesOpen = false;
    state.versionsOpen = false;
    state.compareOpen = false;
    state.timelineOpen = false;
    await loadInsights();
  }
  renderWorkspace();
}

function toggleRevisionMetrics() {
  state.showRevisionMetrics = !state.showRevisionMetrics;
  renderWorkspace();
}

async function toggleCompare() {
  state.compareOpen = !state.compareOpen;
  if (state.compareOpen) {
    state.providersOpen = false;
    state.stylesOpen = false;
    state.candidatesOpen = false;
    state.versionsOpen = false;
    state.insightsOpen = false;
    state.timelineOpen = false;
    if (!state.versionHistory) {
      try {
        state.versionHistory = await invokeHost("get_version_history");
      } catch (error) {
        state.compareOpen = false;
        setNotice("error", normalizeHostError(error).message);
      }
    }
    if (state.compareOpen && state.versionHistory) {
      const snapshots = state.versionHistory.checkpoints;
      const documents = state.workspace.documents;
      const firstSnapshot = snapshots[0]?.id ?? "";
      const secondSnapshot = snapshots[1]?.id ?? snapshots[0]?.id ?? "";
      const firstDocument = documents[0]?.id ?? "";
      state.compareSelection = {
        snapshotIdA: state.compareSelection.snapshotIdA || firstSnapshot,
        snapshotIdB: state.compareSelection.snapshotIdB || secondSnapshot,
        documentIdA: state.compareSelection.documentIdA || firstDocument,
        documentIdB: state.compareSelection.documentIdB || firstDocument,
      };
    }
  } else {
    state.compareResult = null;
  }
  renderWorkspace();
}

async function toggleTimeline() {
  state.timelineOpen = !state.timelineOpen;
  if (state.timelineOpen) {
    state.providersOpen = false;
    state.stylesOpen = false;
    state.candidatesOpen = false;
    state.versionsOpen = false;
    state.insightsOpen = false;
    state.compareOpen = false;
    await loadTimelineEvents();
  } else {
    state.timelineSelected = null;
  }
  renderWorkspace();
}

async function loadTimelineEvents() {
  state.timelineBusy = true;
  renderWorkspace();
  try {
    state.timelineEvents = await invokeHost("list_timeline_events");
  } catch (error) {
    setNotice("error", normalizeHostError(error).message);
    state.timelineOpen = false;
  } finally {
    state.timelineBusy = false;
    renderWorkspace();
  }
}

async function loadInsights() {
  state.insightsBusy = true;
  renderWorkspace();
  try {
    state.insightsData = await invokeHost("get_operation_insights");
  } catch (error) {
    setNotice("error", normalizeHostError(error).message);
  } finally {
    state.insightsBusy = false;
    renderWorkspace();
  }
}

async function exportDiagnostics() {
  try {
    const response = await invokeHost("export_diagnostics");
    setNotice("success", t("export.diagnostics", { path: response.path }));
  } catch (error) {
    setNotice("error", normalizeHostError(error).message);
  } finally {
    renderWorkspace();
  }
}

function insightsDrawer(docFactory = (typeof document !== "undefined" ? document : null)) {
  const data = state.insightsData;
  const drawer = element("aside", { className: "version-drawer insights-drawer", attrs: { role: "dialog", "aria-modal": "true", "aria-labelledby": "insights-drawer-title" } }, [
    element("div", { className: "drawer-heading" }, [
      element("div", {}, [
        element("span", { className: "eyebrow", text: "OPERATION INSIGHTS" }),
        element("h2", { text: t("insights.title"), attrs: { id: "insights-drawer-title" } }),
      ], docFactory),
      iconOnlyButton("close", "icon-button", toggleInsights, { title: t("common.close"), attrs: { "aria-label": t("a11y.closeDrawer") } }, docFactory),
    ], docFactory),
    element("p", {
      className: "drawer-intro",
      text: t("insights.description"),
    }, [], docFactory),
    element("div", { className: "insights-actions" }, [
      button(t("insights.exportDiagnostics"), "small-button", exportDiagnostics, { disabled: state.insightsBusy === true }, docFactory),
      button(state.insightsBusy === true ? t("common.refreshing") : t("common.refresh"), "small-button", loadInsights, { disabled: state.insightsBusy === true }, docFactory),
    ], docFactory),
  ], docFactory);
  attachDrawerKeyboard(drawer, docFactory, () => { state.insightsOpen = false; });
  if (!data) {
    drawer.append(element("p", { className: "drawer-empty", text: state.insightsBusy === true ? t("insights.loading") : t("insights.notLoaded") }, [], docFactory));
    return drawer;
  }
  const summary = data.summary || {};
  const totalRuns = summary.totalRuns ?? 0;
  const totalInputTokens = summary.totalInputTokens ?? 0;
  const totalOutputTokens = summary.totalOutputTokens ?? 0;
  const totalTokens = summary.totalTokens ?? 0;
  const acceptedCount = summary.acceptedCount ?? 0;
  const rejectedCount = summary.rejectedCount ?? 0;
  const conflictedCount = summary.conflictedCount ?? 0;
  const decidedTotal = acceptedCount + rejectedCount + conflictedCount;
  const acceptRate = decidedTotal > 0
    ? `${((acceptedCount / decidedTotal) * 100).toFixed(1)}%`
    : t("insights.noData");
  const fees = estimateFees(data.recentRuns);
  const feeEntries = Object.entries(fees);
  const estimatedFeeTotal = feeEntries.reduce((sum, [, bucket]) => sum + (bucket.fee || 0), 0);
  drawer.append(element("div", { className: "insights-summary-grid" }, [
    element("div", { className: "insights-summary-card" }, [
      element("span", { className: "label", text: t("insights.totalOps") }, [], docFactory),
      element("span", { className: "value", text: totalRuns.toLocaleString("zh-CN") }, [], docFactory),
      element("span", { className: "hint", text: t("insights.totalOpsHint") }, [], docFactory),
    ], docFactory),
    element("div", { className: "insights-summary-card" }, [
      element("span", { className: "label", text: t("insights.inputTokens") }, [], docFactory),
      element("span", { className: "value", text: totalInputTokens.toLocaleString("zh-CN") }, [], docFactory),
      element("span", { className: "hint", text: t("insights.inputTokensHint") }, [], docFactory),
    ], docFactory),
    element("div", { className: "insights-summary-card" }, [
      element("span", { className: "label", text: t("insights.outputTokens") }, [], docFactory),
      element("span", { className: "value", text: totalOutputTokens.toLocaleString("zh-CN") }, [], docFactory),
      element("span", { className: "hint", text: t("insights.outputTokensHint") }, [], docFactory),
    ], docFactory),
    element("div", { className: "insights-summary-card" }, [
      element("span", { className: "label", text: t("insights.totalTokens") }, [], docFactory),
      element("span", { className: "value", text: totalTokens.toLocaleString("zh-CN") }, [], docFactory),
      element("span", { className: "hint", text: t("insights.totalTokensHint") }, [], docFactory),
    ], docFactory),
    element("div", { className: "insights-summary-card" }, [
      element("span", { className: "label", text: t("insights.acceptRate") }, [], docFactory),
      element("span", { className: "value", text: acceptRate }, [], docFactory),
      element("span", { className: "hint", text: t("insights.acceptRateHint", { accepted: acceptedCount, rejected: rejectedCount, conflicted: conflictedCount }) }, [], docFactory),
    ], docFactory),
    element("div", { className: "insights-summary-card" }, [
      element("span", { className: "label", text: t("insights.estimatedFee") }, [], docFactory),
      element("span", { className: "value", text: `$${estimatedFeeTotal.toFixed(4)}` }, [], docFactory),
      element("span", { className: "hint", text: t("insights.estimatedFeeHint") }, [], docFactory),
    ], docFactory),
  ], docFactory));
  if (data.revisionMetrics) {
    drawer.append(element("div", { className: "insights-revision-actions" }, [
      button(
        state.showRevisionMetrics ? t("insights.hideRevisionRate") : t("insights.showRevisionRate"),
        "small-button",
        toggleRevisionMetrics,
        {},
        docFactory,
      ),
    ], docFactory));
    if (state.showRevisionMetrics) {
      const rm = data.revisionMetrics;
      const rate = typeof rm.acceptedAfterRejectionRate === "number" ? rm.acceptedAfterRejectionRate : 0;
      const accepted = rm.acceptedAfterRejection ?? 0;
      const total = rm.totalProposalsAccepted ?? 0;
      const ratePct = (rate * 100).toFixed(1);
      const rateColor = rate < 0.1 ? "#10b981" : rate <= 0.25 ? "#eab308" : "#ef4444";
      drawer.append(element("div", { className: "insights-revision-card" }, [
        element("span", { className: "label", text: t("insights.revisionRate") }, [], docFactory),
        element("span", {
          className: "value",
          text: `${ratePct}% · ${accepted}/${total}`,
          attrs: { style: `color: ${rateColor};` },
        }, [], docFactory),
        element("span", { className: "hint", text: t("insights.revisionRateHint") }, [], docFactory),
      ], docFactory));
    }
    // v0.7.0 Stage 3 (D4): payload 变形率卡片与二次编辑率卡片共用
    // showRevisionMetrics 开关（不新增开关），满足 D5 "默认隐藏"约束。
    // rate 颜色编码用 hex（参考 v0.5.0 fpsMonitor 模式）：
    // 低(绿 #10b981, <5%)、中(黄 #eab308, 5-15%)、高(红 #ef4444, >15%)。
    if (state.showRevisionMetrics && data.payloadHashVariations) {
      const phv = data.payloadHashVariations;
      const phvRate = typeof phv.variationRate === "number" ? phv.variationRate : 0;
      const phvVariation = phv.proposalsWithVariation ?? 0;
      const phvTotal = phv.totalProposals ?? 0;
      const phvRatePct = (phvRate * 100).toFixed(1);
      const phvRateColor = phvRate < 0.05 ? "#10b981" : phvRate <= 0.15 ? "#eab308" : "#ef4444";
      drawer.append(element("div", { className: "insights-payload-card" }, [
        element("span", { className: "label", text: t("insights.payloadDrift") }, [], docFactory),
        element("span", {
          className: "value",
          text: `${phvRatePct}% · ${phvVariation}/${phvTotal}`,
          attrs: { style: `color: ${phvRateColor};` },
        }, [], docFactory),
        element("span", { className: "hint", text: t("insights.payloadDriftHint") }, [], docFactory),
      ], docFactory));
    }
  }
  if (data.generatedAt) {
    drawer.append(element("p", { className: "insights-generated-at", text: t("insights.generatedAt", { date: formatDate(data.generatedAt) }) }, [], docFactory));
  }
  if (feeEntries.length) {
    const feeTable = element("div", { className: "insights-fee-table" }, [
      element("div", { className: "insights-fee-row insights-fee-header" }, [
        element("span", { text: "Provider" }, [], docFactory),
        element("span", { text: t("insights.colInputTokens") }, [], docFactory),
        element("span", { text: t("insights.colOutputTokens") }, [], docFactory),
        element("span", { text: t("insights.colCachedTokens") }, [], docFactory),
        element("span", { text: t("insights.colEstimatedFee") }, [], docFactory),
      ], docFactory),
      ...feeEntries.map(([providerId, bucket]) => element("div", { className: "insights-fee-row" }, [
        element("span", { text: providerId }, [], docFactory),
        element("span", { text: bucket.inputTokens.toLocaleString("zh-CN") }, [], docFactory),
        element("span", { text: bucket.outputTokens.toLocaleString("zh-CN") }, [], docFactory),
        element("span", { text: bucket.cachedInputTokens.toLocaleString("zh-CN") }, [], docFactory),
        element("span", { text: `$${(bucket.fee || 0).toFixed(4)}` }, [], docFactory),
      ], docFactory)),
    ], docFactory);
    drawer.append(feeTable);
  }
  drawer.append(element("h3", { text: t("insights.recentOps") }, [], docFactory));
  const recentRuns = Array.isArray(data.recentRuns) ? data.recentRuns.slice(0, 20) : [];
  if (!recentRuns.length) {
    drawer.append(element("p", { className: "drawer-empty", text: t("insights.recentOpsEmpty") }, [], docFactory));
  } else {
    drawer.append(element("div", { className: "insights-run-list" }, recentRuns.map((run) => element("div", { className: "insights-run-row" }, [
      element("span", { className: `state-pill state-${run.state}`, text: insightsStateLabel(run.state) }, [], docFactory),
      element("span", { className: "run-provider", text: run.providerId ?? "—" }, [], docFactory),
      element("span", { className: "run-time", text: run.startedAt ? formatDate(run.startedAt) : "—" }, [], docFactory),
      element("span", { className: "run-tokens", text: run.totalTokens === null || run.totalTokens === undefined ? "—" : run.totalTokens.toLocaleString("zh-CN") }, [], docFactory),
    ], docFactory)), docFactory));
  }
  return drawer;
}

function insightsStateLabel(state) {
  return {
    accepted: t("insights.accepted"),
    rejected: t("insights.rejected"),
    conflicted: t("insights.conflicted"),
  }[state] || state || "—";
}

function timelineDrawer(docFactory = (typeof document !== "undefined" ? document : null)) {
  const data = state.timelineEvents;
  const drawer = element("aside", { className: "version-drawer timeline-drawer", attrs: { role: "dialog", "aria-modal": "true", "aria-labelledby": "timeline-drawer-title" } }, [
    element("div", { className: "drawer-heading" }, [
      element("div", {}, [
        element("span", { className: "eyebrow", text: "VERSION TIMELINE" }, [], docFactory),
        element("h2", { text: t("timelineDrawer.title"), attrs: { id: "timeline-drawer-title" } }, [], docFactory),
      ], docFactory),
      iconOnlyButton("close", "icon-button", toggleTimeline, { title: t("common.close"), attrs: { "aria-label": t("a11y.closeDrawer") } }, docFactory),
    ], docFactory),
    element("p", {
      className: "drawer-intro",
      text: t("timelineDrawer.description"),
    }, [], docFactory),
  ], docFactory);
  attachDrawerKeyboard(drawer, docFactory, () => {
    state.timelineOpen = false;
    state.timelineSelected = null;
  });

  if (state.timelineBusy) {
    drawer.append(element("p", { className: "drawer-empty", text: t("timelineDrawer.loading") }, [], docFactory));
    return drawer;
  }
  if (!data || !Array.isArray(data.events) || !data.events.length) {
    drawer.append(element("p", { className: "drawer-empty", text: t("timelineDrawer.empty") }, [], docFactory));
    return drawer;
  }

  const events = data.events;
  drawer.append(element("h3", { text: t("timelineDrawer.nodeCount", { count: events.length }) }, [], docFactory));

  // v0.7.0 Stage 1 (D1 + D5): assign each event to a distinct horizontal
  // lane so that branch forks and merge points can be visually
  // distinguished. The lane map is consulted by `timelineNodeRow` (via the
  // closure-bound `laneByCommitId`) and by the SVG overlay below.
  const laneByCommitId = assignTimelineLanes(events);
  const hasMerge = events.some((event) => (event.parentCommitIds ?? []).length > 1);

  const layout = element("div", { className: "timeline-layout" }, [], docFactory);
  const viewport = element("div", { className: "timeline-list compare-drawer-virtual" }, [], docFactory);
  layout.append(viewport);

  // Bind the lane lookup into the row renderer via a closure so the
  // virtualiser still sees the standard (event, index, docFactory) row
  // signature it expects.
  const rowRenderer = (event, index, doc) => timelineNodeRow(event, index, doc, laneByCommitId);
  const controller = virtualizeDiffRows(events, viewport, docFactory, rowRenderer, {
    overscan: 5,
    estimateHeight: 64,
    viewportHeight: 600,
  });
  viewport.__virtualController = controller;
  viewport.addEventListener("scroll", () => {
    controller.update(viewport.scrollTop || 0, viewport.clientHeight || 600);
  });
  controller.update(0, viewport.clientHeight || 600);

  // v0.7.0 Stage 1 (D7): SVG overlay. Render an <svg> overlay sitting on
  // top of the timeline list viewport. Each <path> encodes a connection
  // between a merge commit and one of its non-mainline parents (position 1
  // onwards). The overlay is only appended when at least one merge commit
  // exists so that purely linear histories remain visually identical to
  // the v0.6.0 baseline (no empty SVG element in the DOM tree).
  if (hasMerge) {
    const overlay = buildTimelineSvgOverlay(events, laneByCommitId, docFactory);
    if (overlay) viewport.append(overlay);
  }

  const preview = element("div", { className: "timeline-preview" }, [], docFactory);
  const selected = state.timelineSelected;
  if (!selected) {
    preview.append(element("div", { className: "timeline-preview-empty", text: t("timelineDrawer.previewEmpty") }, [], docFactory));
  } else {
    const opState = selected.operationSummary?.state;
    const stateLabel = opState ? (TIMELINE_STATE_LABELS[opState] || opState) : t("timelineDrawer.noOperation");
    const stateColor = opState ? TIMELINE_STATE_COLORS[opState] : "#9ca3af";
    const statePillClass = opState
      ? `timeline-state-pill timeline-state-${opState}`
      : "timeline-state-pill timeline-state-none";
    preview.append(element("div", { className: "timeline-preview-detail" }, [
      element("div", { className: "timeline-preview-row" }, [
        element("span", { className: "timeline-preview-label", text: t("timelineDrawer.createdAt") }, [], docFactory),
        element("span", { text: formatDate(selected.createdAt), title: selected.commitId }, [], docFactory),
      ], docFactory),
      element("div", { className: "timeline-preview-row" }, [
        element("span", { className: "timeline-preview-label", text: t("timelineDrawer.operationState") }, [], docFactory),
        element("span", {
          className: statePillClass,
          text: stateLabel,
          attrs: { style: `color: ${stateColor};` },
        }, [], docFactory),
      ], docFactory),
    ], docFactory));
  }
  layout.append(preview);
  drawer.append(layout);
  return drawer;
}

/**
 * v0.7.0 Stage 1 (D5): assign each event to a distinct horizontal lane
 * offset so that branch forks and merge points can be visually
 * distinguished. The algorithm is intentionally simple ("轨道分配" per
 * the design spec) and does not invoke GitGraph.js or any complex layout
 * solver:
 *
 * 1. Iterate events in the order they appear in the timeline (already
 *    sorted by `created_at` descending on the host side).
 * 2. The first parent of each commit (position 0) inherits the lane of
 *    the parent — this represents the mainline and keeps the trunk on
 *    lane 0.
 * 3. Subsequent parents (position >= 1) indicate a branch merge. We
 *    allocate a fresh lane for each such parent by scanning the lane pool
 *    for the lowest free index < 10 (per the D5 constraint "轨道数 < 10")
 *    and assigning it to the parent commit.
 *
 * The function returns a `Map<string, number>` mapping commit id to lane
 * index. Commits not present in the map default to lane 0.
 *
 * @param {Array<{commitId: string, parentCommitIds?: string[]}>} events
 * @returns {Map<string, number>}
 */
function assignTimelineLanes(events) {
  const laneByCommitId = new Map();
  /** Lanes currently in use by active branches. `Set<number>` so we can
   * cheaply test membership and pick the lowest free index. */
  const activeLanes = new Set([0]);
  // Pass 1: scan merge commits first (any order) so that non-mainline
  // parents get a fresh lane BEFORE the linear inheritance pass below
  // would otherwise collapse them onto lane 0. This two-pass approach is
  // necessary because the events array is sorted by `created_at` descending
  // (newest first), so a merge commit is typically visited before its
  // branch fork parent — but the parent's own event row would inherit the
  // mainline lane if we processed it in a single pass. By pre-allocating
  // branch lanes in pass 1, pass 2 can skip already-assigned commits and
  // preserve the fork topology.
  for (const event of events) {
    const parents = event.parentCommitIds ?? [];
    if (parents.length <= 1) continue; // linear commit, no branch allocation
    for (let i = 1; i < parents.length && activeLanes.size < 10; i++) {
      const branchParent = parents[i];
      if (laneByCommitId.has(branchParent)) continue; // already assigned
      // Pick the lowest free lane index in [0, 10).
      let lane = 0;
      while (activeLanes.has(lane) && lane < 10) lane++;
      if (lane >= 10) break; // D5 cap reached; remaining parents share lane 0
      laneByCommitId.set(branchParent, lane);
      activeLanes.add(lane);
    }
  }
  // Pass 2: linear inheritance. Each commit not already assigned a lane
  // (via pass 1) inherits its first parent's lane so the mainline stays on
  // lane 0. Seed commits (zero parents) default to lane 0.
  for (const event of events) {
    const commitId = event.commitId;
    if (laneByCommitId.has(commitId)) continue; // already assigned in pass 1
    const parents = event.parentCommitIds ?? [];
    if (parents.length === 0) {
      laneByCommitId.set(commitId, 0);
      continue;
    }
    const firstParent = parents[0];
    const parentLane = laneByCommitId.get(firstParent) ?? 0;
    laneByCommitId.set(commitId, parentLane);
  }
  return laneByCommitId;
}

/**
 * v0.7.0 Stage 1 (D7): build the SVG overlay element containing one <path>
 * per non-mainline branch merge connection. The overlay is an <svg> element
 * created in the SVG namespace so that CSS and querySelector can address
 * its <path> children. Connection coordinates are computed from the lane
 * assignments (D5) and the event index (vertical position) using a simple
 * deterministic grid so the overlay stays in sync with the virtualised
 * list above it.
 *
 * @param {Array<{commitId: string, parentCommitIds?: string[]}>} events
 * @param {Map<string, number>} laneByCommitId
 * @param {Document} docFactory
 * @returns {SVGSVGElement | null}
 */
function buildTimelineSvgOverlay(events, laneByCommitId, docFactory) {
  // Collect merge connections: each entry is { fromLane, fromY, toLane, toY }
  // representing a <path> from a merge commit to one of its non-mainline
  // parents. Only parents at position >= 1 produce a connection; the
  // mainline (position 0) is rendered implicitly by the vertical alignment
  // of the timeline nodes themselves.
  const connections = [];
  const eventIndexByCommitId = new Map();
  events.forEach((event, index) => {
    eventIndexByCommitId.set(event.commitId, index);
  });
  for (const event of events) {
    const parents = event.parentCommitIds ?? [];
    if (parents.length <= 1) continue; // linear commit, no merge connection
    const mergeIndex = eventIndexByCommitId.get(event.commitId) ?? 0;
    const mergeLane = laneByCommitId.get(event.commitId) ?? 0;
    const mergeY = mergeIndex * 64 + 32; // matches `estimateHeight` (64) + half-height
    for (let i = 1; i < parents.length; i++) {
      const branchParent = parents[i];
      const parentIndex = eventIndexByCommitId.get(branchParent);
      if (parentIndex === undefined) continue; // parent not in timeline (e.g. not a checkpoint)
      const parentLane = laneByCommitId.get(branchParent) ?? 0;
      const parentY = parentIndex * 64 + 32;
      connections.push({
        fromLane: mergeLane,
        fromY: mergeY,
        toLane: parentLane,
        toY: parentY,
      });
    }
  }
  if (connections.length === 0) return null;
  const laneWidth = 24; // px per lane column
  const overlayWidth = 10 * laneWidth; // D5 cap (10 lanes)
  const overlay = element("svg:svg", {
    className: "timeline-svg-overlay",
    attrs: {
      "data-lane-width": String(laneWidth),
      "data-connections": String(connections.length),
      "aria-hidden": "true",
      viewBox: `0 0 ${overlayWidth} 100`,
      preserveAspectRatio: "none",
    },
  }, [], docFactory);
  for (const connection of connections) {
    const fromX = connection.fromLane * laneWidth + laneWidth / 2;
    const toX = connection.toLane * laneWidth + laneWidth / 2;
    const fromY = connection.fromY;
    const toY = connection.toY;
    // Cubic bezier curve so the connection visually resembles a GitGraph
    // branch line without introducing the full GitGraph.js layout solver
    // (D5 constraint). The control points sit at the midpoint Y so the
    // curve has a smooth horizontal bend.
    const midY = (fromY + toY) / 2;
    const path = element("svg:path", {
      className: "timeline-lane timeline-lane-merge",
      attrs: {
        d: `M ${fromX} ${fromY} C ${fromX} ${midY}, ${toX} ${midY}, ${toX} ${toY}`,
        "data-from-lane": String(connection.fromLane),
        "data-to-lane": String(connection.toLane),
      },
    }, [], docFactory);
    overlay.append(path);
  }
  return overlay;
}

function timelineNodeRow(event, _index, docFactory = (typeof document !== "undefined" ? document : null), laneByCommitId = new Map()) {
  const opState = event.operationSummary?.state;
  const stateLabel = opState ? (TIMELINE_STATE_LABELS[opState] || opState) : t("timelineDrawer.noOperationShort");
  const stateColor = opState ? TIMELINE_STATE_COLORS[opState] : "#9ca3af";
  const statePillClass = opState
    ? `timeline-state-pill timeline-state-${opState}`
    : "timeline-state-pill timeline-state-none";
  // v0.7.0 Stage 1 (D5): surface the lane assignment via a data attribute
  // so CSS can apply horizontal padding per lane, and tests can verify that
  // forks and merges receive distinct lane indices.
  const lane = laneByCommitId.get(event.commitId) ?? 0;
  const node = element("article", {
    className: "timeline-node compare-block",
    attrs: {
      "data-checkpoint-id": event.checkpointId,
      "data-commit-id": event.commitId,
      "data-lane": String(lane),
    },
  }, [
    element("div", { className: "timeline-node-heading" }, [
      element("span", { className: "timeline-node-dot", attrs: { style: `background: ${stateColor};` } }, [], docFactory),
      element("span", { className: "timeline-node-time", text: formatDate(event.createdAt) }, [], docFactory),
      element("span", {
        className: statePillClass,
        text: stateLabel,
        attrs: { style: `color: ${stateColor};` },
      }, [], docFactory),
    ], docFactory),
  ], docFactory);
  node.addEventListener("click", () => {
    state.timelineSelected = event;
    renderWorkspace();
  });
  return node;
}

function getRAF(docFactory, options) {
  return options?.requestAnimationFrame
    || docFactory?.defaultView?.requestAnimationFrame
    || (typeof requestAnimationFrame !== "undefined" ? requestAnimationFrame : null)
    || ((cb) => setTimeout(() => cb(Date.now()), 0));
}

function getCAF(docFactory, options) {
  return options?.cancelAnimationFrame
    || docFactory?.defaultView?.cancelAnimationFrame
    || (typeof cancelAnimationFrame !== "undefined" ? cancelAnimationFrame : null)
    || ((id) => clearTimeout(id));
}

function getResizeObserverCtor(docFactory, options) {
  return options?.ResizeObserver
    || docFactory?.defaultView?.ResizeObserver
    || (typeof ResizeObserver !== "undefined" ? ResizeObserver : null);
}

// Virtualizes a list of diff blocks inside a scrollable container using
// position:absolute + top offset. Renders only visible rows + overscan.
// Returns a controller { update, destroy, ...introspection }.
function virtualizeDiffRows(blocks, container, docFactory, renderRow, options = {}) {
  const blocksList = Array.isArray(blocks) ? blocks : [];
  const overscan = options.overscan ?? 5;
  const estimateHeight = options.estimateHeight ?? 48;
  const rowHeightCache = new Map(); // index -> measured height
  const offsetCache = new Map();    // index -> top offset (memoized)
  const renderedNodes = new Map();  // index -> element
  const observers = [];
  let track = null;
  let lastScrollTop = 0;
  let lastViewportHeight = options.viewportHeight ?? 600;
  let pendingRAF = null;

  function heightOf(index) {
    return rowHeightCache.get(index) ?? estimateHeight;
  }

  function offsetFor(index) {
    if (offsetCache.has(index)) return offsetCache.get(index);
    let top = 0;
    for (let i = 0; i < index; i++) top += heightOf(i);
    offsetCache.set(index, top);
    return top;
  }

  function totalHeight() {
    let total = 0;
    for (let i = 0; i < blocksList.length; i++) total += heightOf(i);
    return total;
  }

  // Binary search: find first index whose top + height > scrollTop.
  // O(log n) in offsetCache lookups (offsets memoized).
  function findFirstVisible(scrollTop) {
    let lo = 0;
    let hi = blocksList.length;
    while (lo < hi) {
      const mid = (lo + hi) >>> 1;
      if (offsetFor(mid) + heightOf(mid) <= scrollTop) {
        lo = mid + 1;
      } else {
        hi = mid;
      }
    }
    return lo;
  }

  function computeVisibleRange(scrollTop, viewportHeight) {
    if (!blocksList.length) return { start: 0, end: 0 };
    const firstVisible = findFirstVisible(scrollTop);
    const start = Math.max(0, firstVisible - overscan);
    let end = firstVisible;
    let cursor = offsetFor(firstVisible);
    const limit = scrollTop + viewportHeight;
    while (end < blocksList.length && cursor < limit) {
      cursor += heightOf(end);
      end++;
    }
    return { start, end: Math.min(blocksList.length, end + overscan) };
  }

  function ensureTrack() {
    if (track) return track;
    track = element("div", { className: "compare-drawer-virtual-track" }, [], docFactory);
    container.append(track);
    return track;
  }

  function measureNode(node, entry) {
    if (entry?.contentRect && typeof entry.contentRect.height === "number" && entry.contentRect.height > 0) {
      return entry.contentRect.height;
    }
    const rect = node.getBoundingClientRect?.();
    if (rect && typeof rect.height === "number" && rect.height > 0) return rect.height;
    if (typeof node.offsetHeight === "number" && node.offsetHeight > 0) return node.offsetHeight;
    return estimateHeight;
  }

  function attachObserver(index, node) {
    const ROCtor = getResizeObserverCtor(docFactory, options);
    if (!ROCtor) return null;
    const observer = new ROCtor((entries) => {
      for (const entry of entries) {
        const target = entry?.target || node;
        const newHeight = measureNode(target, entry);
        if (newHeight && newHeight !== rowHeightCache.get(index)) {
          rowHeightCache.set(index, newHeight);
          offsetCache.clear();
          // Re-render with the latest known scroll position.
          scheduleRender(lastScrollTop, lastViewportHeight);
        }
      }
    });
    observer.observe(node);
    observers.push(observer);
    return observer;
  }

  function doRender(scrollTop, viewportHeight) {
    const trackEl = ensureTrack();
    const total = totalHeight();
    trackEl.style.height = `${total}px`;
    const { start, end } = computeVisibleRange(scrollTop, viewportHeight);

    // Remove rows outside [start, end).
    for (const [idx, node] of renderedNodes) {
      if (idx < start || idx >= end) {
        node.remove();
        renderedNodes.delete(idx);
      }
    }

    // Render (or reposition) rows in [start, end).
    for (let i = start; i < end; i++) {
      let node = renderedNodes.get(i);
      if (!node) {
        node = renderRow(blocksList[i], i, docFactory);
        node.style.position = "absolute";
        node.style.left = "0";
        node.style.right = "0";
        node.style.top = `${offsetFor(i)}px`;
        attachObserver(i, node);
        renderedNodes.set(i, node);
        trackEl.append(node);
      } else {
        node.style.top = `${offsetFor(i)}px`;
      }
    }
  }

  function scheduleRender(scrollTop, viewportHeight) {
    lastScrollTop = scrollTop;
    lastViewportHeight = viewportHeight;
    if (pendingRAF !== null) return;
    const raf = getRAF(docFactory, options);
    const caf = getCAF(docFactory, options);
    pendingRAF = raf(() => {
      pendingRAF = null;
      doRender(lastScrollTop, lastViewportHeight);
    });
    // Stash cancel handle for destroy().
    scheduleRender._caf = caf;
  }

  function destroy() {
    if (pendingRAF !== null) {
      const caf = scheduleRender._caf || getCAF(docFactory, options);
      try { caf(pendingRAF); } catch { /* noop */ }
      pendingRAF = null;
    }
    for (const observer of observers) {
      try { observer.disconnect?.(); } catch { /* noop */ }
    }
    observers.length = 0;
    renderedNodes.clear();
    offsetCache.clear();
  }

  return {
    update: scheduleRender,
    renderImmediate: doRender,
    destroy,
    // Introspection helpers for tests / dev tooling.
    _rowHeightCache: rowHeightCache,
    _offsetCache: offsetCache,
    _renderedNodes: renderedNodes,
    _findFirstVisible: findFirstVisible,
    _computeVisibleRange: computeVisibleRange,
    _offsetFor: offsetFor,
    _totalHeight: totalHeight,
  };
}

// FPS overlay shown only in dev mode (window.__OPTIMIZER_DEV__). Updates every
// ~500ms with color-coded text (>=55 green / >=30 orange / <30 red).
function fpsMonitor(docFactory, options = {}) {
  const devFlag = typeof window !== "undefined" && window !== null
    ? window.__OPTIMIZER_DEV__
    : null;
  if (!devFlag) return null;
  const raf = options.requestAnimationFrame
    || docFactory?.defaultView?.requestAnimationFrame
    || (typeof requestAnimationFrame !== "undefined" ? requestAnimationFrame : null);
  const perf = options.performance
    || (typeof performance !== "undefined" ? performance : { now: () => Date.now() });
  if (!raf || !perf) return null;
  const indicator = element("div", { className: "fps-monitor" }, [], docFactory);
  let lastTime = perf.now ? perf.now() : Date.now();
  let frames = 0;
  function loop(now) {
    frames++;
    if (now - lastTime >= 500) {
      const fps = Math.round((frames * 1000) / (now - lastTime));
      indicator.textContent = `${fps} FPS`;
      indicator.style.color = fps >= 55 ? "#16a34a" : fps >= 30 ? "#d97706" : "#dc2626";
      frames = 0;
      lastTime = now;
    }
    raf(loop);
  }
  raf(loop);
  return indicator;
}

// v0.8.0 Stage 3 (a11y): linkedom's HTMLSelectElement.value is a getter-only
// property, so assigning .value throws in unit tests. Real browsers implement
// the setter. This helper tries the native setter first and falls back to
// marking the matching <option> as selected so the initial state is correct
// in both environments.
function setSelectValue(selectEl, value) {
  try {
    selectEl.value = value;
    return;
  } catch {
    // linkedom getter-only — fall through to option marking.
  }
  const options = selectEl.options || [];
  for (const option of options) {
    if (option.value === value) {
      option.setAttribute("selected", "selected");
    } else {
      option.removeAttribute("selected");
    }
  }
}

function compareDrawer(docFactory = (typeof document !== "undefined" ? document : null)) {
  const history = state.versionHistory;
  const documents = state.workspace.documents;
  const drawer = element("aside", { className: "version-drawer compare-drawer", attrs: { role: "dialog", "aria-modal": "true", "aria-labelledby": "compare-drawer-title" } }, [
    element("div", { className: "drawer-heading" }, [
      element("div", {}, [
        element("span", { className: "eyebrow", text: "DOCUMENT DIFF" }),
        element("h2", { text: t("compare.title"), attrs: { id: "compare-drawer-title" } }),
      ]),
      iconOnlyButton("close", "icon-button", toggleCompare, { title: t("compare.close"), attrs: { "aria-label": t("a11y.closeDrawer") } }),
    ]),
    element("p", {
      className: "drawer-intro",
      text: t("compare.description"),
    }),
  ]);
  attachDrawerKeyboard(drawer, docFactory, () => { state.compareOpen = false; state.compareResult = null; });
  if (!history) {
    drawer.append(element("p", { className: "drawer-empty", text: t("compare.loading") }));
    return drawer;
  }
  if (!history.checkpoints.length) {
    drawer.append(element("p", { className: "drawer-empty", text: t("compare.noCheckpoint") }));
    return drawer;
  }
  const selection = state.compareSelection;
  const snapshotOptions = history.checkpoints.map((checkpoint) =>
    element("option", {
      value: checkpoint.id,
      text: formatDate(checkpoint.createdAt),
    }),
  );
  const snapshotASelect = element("select", { name: "snapshotIdA", attrs: { "aria-label": t("compare.snapshotA") } }, snapshotOptions);
  setSelectValue(snapshotASelect, selection.snapshotIdA);
  snapshotASelect.addEventListener("change", () => {
    state.compareSelection.snapshotIdA = snapshotASelect.value;
    state.compareResult = null;
  });
  const snapshotBSelect = element("select", { name: "snapshotIdB", attrs: { "aria-label": t("compare.snapshotB") } },
    history.checkpoints.map((checkpoint) =>
      element("option", {
        value: checkpoint.id,
        text: formatDate(checkpoint.createdAt),
      }),
    ),
  );
  setSelectValue(snapshotBSelect, selection.snapshotIdB);
  snapshotBSelect.addEventListener("change", () => {
    state.compareSelection.snapshotIdB = snapshotBSelect.value;
    state.compareResult = null;
  });
  const documentASelect = element("select", { name: "documentIdA", attrs: { "aria-label": t("compare.documentA") } },
    documents.map((document) => element("option", { value: document.id, text: document.title })),
  );
  setSelectValue(documentASelect, selection.documentIdA);
  documentASelect.addEventListener("change", () => {
    state.compareSelection.documentIdA = documentASelect.value;
    state.compareResult = null;
  });
  const documentBSelect = element("select", { name: "documentIdB", attrs: { "aria-label": t("compare.documentB") } },
    documents.map((document) => element("option", { value: document.id, text: document.title })),
  );
  setSelectValue(documentBSelect, selection.documentIdB);
  documentBSelect.addEventListener("change", () => {
    state.compareSelection.documentIdB = documentBSelect.value;
    state.compareResult = null;
  });
  const sameSnapshot = selection.snapshotIdA === selection.snapshotIdB
    && selection.documentIdA === selection.documentIdB;
  const canCompare = Boolean(
    selection.snapshotIdA
      && selection.snapshotIdB
      && selection.documentIdA
      && selection.documentIdB
      && !sameSnapshot
      && !state.compareBusy,
  );
  const form = element("form", { className: "compare-form" }, [
    element("div", { className: "compare-form-grid" }, [
      element("label", { className: "field" }, [
        element("span", { text: t("compare.snapshotA") }),
        snapshotASelect,
      ]),
      element("label", { className: "field" }, [
        element("span", { text: t("compare.snapshotB") }),
        snapshotBSelect,
      ]),
      element("label", { className: "field" }, [
        element("span", { text: t("compare.documentA") }),
        documentASelect,
      ]),
      element("label", { className: "field" }, [
        element("span", { text: t("compare.documentB") }),
        documentBSelect,
      ]),
    ]),
    sameSnapshot
      ? element("p", { className: "form-hint compare-hint-warning", text: t("compare.sameSelection") })
      : null,
    button(
      state.compareBusy ? t("compare.generating") : t("compare.generate"),
      "primary-button compare-submit",
      runCompareDocuments,
      { disabled: !canCompare },
    ),
  ]);
  form.addEventListener("submit", (event) => {
    event.preventDefault();
    void runCompareDocuments();
  });
  drawer.append(form);
  if (state.compareResult) {
    drawer.append(compareResultView(state.compareResult, docFactory));
  } else if (state.compareBusy) {
    drawer.append(element("p", { className: "drawer-empty", text: t("compare.comparing") }, [], docFactory));
  }
  return drawer;
}

async function runCompareDocuments() {
  if (state.compareBusy) return;
  const selection = state.compareSelection;
  if (!selection.snapshotIdA || !selection.snapshotIdB || !selection.documentIdA || !selection.documentIdB) return;
  if (
    selection.snapshotIdA === selection.snapshotIdB
    && selection.documentIdA === selection.documentIdB
  ) return;
  state.compareBusy = true;
  state.compareResult = null;
  renderWorkspace();
  try {
    const result = await invokeHost("compare_documents", {
      input: {
        schemaVersion: 1,
        snapshotIdA: selection.snapshotIdA,
        snapshotIdB: selection.snapshotIdB,
        documentIdA: selection.documentIdA,
        documentIdB: selection.documentIdB,
      },
    });
    state.compareResult = result;
  } catch (error) {
    setNotice("error", normalizeHostError(error).message);
  } finally {
    state.compareBusy = false;
    renderWorkspace();
  }
}

function compareResultView(result, docFactory = (typeof document !== "undefined" ? document : null)) {
  const summary = result.summary || {};
  const blocks = Array.isArray(result.blocks) ? result.blocks : [];
  const summaryChips = [
    { label: t("compare.added"), value: summary.addedCount ?? 0, className: "compare-chip-added" },
    { label: t("compare.removed"), value: summary.removedCount ?? 0, className: "compare-chip-removed" },
    { label: t("compare.modified"), value: summary.modifiedCount ?? 0, className: "compare-chip-modified" },
    { label: t("compare.unchanged"), value: summary.unchangedCount ?? 0, className: "compare-chip-unchanged" },
  ];
  const section = element("section", { className: "compare-result" }, [
    element("div", { className: "compare-summary" }, summaryChips.map((chip) =>
      element("span", { className: `compare-chip ${chip.className}` }, [
        element("strong", { text: String(chip.value) }, [], docFactory),
        element("span", { text: chip.label }, [], docFactory),
      ], docFactory),
    ), docFactory),
    element("h3", { text: t("compare.blockSequence", { count: blocks.length }) }, [], docFactory),
  ], docFactory);
  if (!blocks.length) {
    section.append(element("p", { className: "drawer-empty", text: t("compare.noBlocks") }, [], docFactory));
    return section;
  }
  // Virtualized scroll viewport: keeps the same visual structure (heading +
  // list of block rows) but renders only the visible slice + overscan so
  // 1000+ diff rows stay responsive. The container element is passed to
  // virtualizeDiffRows which manages an inner track + absolutely positioned
  // rows. Scroll position is read from the viewport element.
  const viewport = element("div", { className: "compare-drawer-virtual compare-block-list" }, [], docFactory);
  section.append(viewport);
  const controller = virtualizeDiffRows(blocks, viewport, docFactory, compareBlockRow, {
    overscan: 5,
    estimateHeight: 48,
    viewportHeight: 600,
  });
  // Store controller on the viewport for dev introspection and so future
  // re-renders can destroy it (compareDrawer is rebuilt top-down on render).
  viewport.__virtualController = controller;
  viewport.addEventListener("scroll", () => {
    controller.update(viewport.scrollTop || 0, viewport.clientHeight || 600);
  });
  // Initial render with scrollTop=0. Use requestAnimationFrame via the
  // controller's throttled update so we don't block the current paint.
  controller.update(0, viewport.clientHeight || 600);
  return section;
}

function compareBlockRow(entry, _index, docFactory = (typeof document !== "undefined" ? document : null)) {
  const kind = entry.kind || "unchanged";
  const labels = {
    added: t("compare.added"),
    removed: t("compare.removed"),
    modified: t("compare.modified"),
    unchanged: t("compare.unchanged"),
  };
  const idLabel = entry.blockIdA && entry.blockIdB
    ? `${entry.blockIdA.slice(0, 10)} ↔ ${entry.blockIdB.slice(0, 10)}`
    : entry.blockIdA
      ? `${entry.blockIdA.slice(0, 12)} · ${t("compare.onlyLeft")}`
      : `${entry.blockIdB?.slice(0, 12) ?? ""} · ${t("compare.onlyRight")}`;
  const row = element("article", { className: `compare-block compare-block-${kind}` }, [
    element("div", { className: "compare-block-heading" }, [
      element("span", { className: `compare-kind-pill compare-kind-${kind}`, text: labels[kind] || kind }, [], docFactory),
      element("code", { text: idLabel }, [], docFactory),
    ], docFactory),
  ], docFactory);
  if (entry.textDiff && Array.isArray(entry.textDiff) && entry.textDiff.length) {
    const diff = element("div", { className: "compare-text-diff" }, [], docFactory);
    for (const op of entry.textDiff) {
      if (op.equal !== undefined) {
        diff.append(element("span", { className: "diff-equal", text: op.equal }, [], docFactory));
      } else if (op.delete !== undefined) {
        diff.append(element("del", { className: "diff-delete", text: op.delete }, [], docFactory));
      } else if (op.insert !== undefined) {
        diff.append(element("ins", { className: "diff-insert", text: op.insert }, [], docFactory));
      }
    }
    row.append(diff);
  }
  return row;
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
        element("h2", { text: t("styles.title") }),
      ]),
      button("×", "icon-button", toggleStyles, { title: t("common.close") }),
    ]),
    element("p", {
      className: "style-help",
      text: t("styles.description"),
    }),
    knowledgeForm(),
    element("h3", { text: t("styles.canonicalCount", { count: canonicalKnowledge.length }) }),
  ]);
  if (!canonicalKnowledge.length) {
    drawer.append(element("p", { className: "drawer-empty", text: t("styles.canonicalEmpty") }));
  }
  for (const item of canonicalKnowledge) drawer.append(knowledgeItemCard(item));
  if (inactiveKnowledge.length) {
    drawer.append(element("h3", { text: t("styles.inactiveCount", { count: inactiveKnowledge.length }) }));
    for (const item of inactiveKnowledge) drawer.append(knowledgeItemCard(item));
  }
  drawer.append(element("h3", { text: t("styles.activeCount", { count: active.length }) }));
  if (!active.length) {
    drawer.append(element("p", { className: "drawer-empty", text: t("styles.activeEmpty") }));
  }
  for (const sample of active) drawer.append(styleSampleCard(sample));
  if (archived.length) {
    drawer.append(element("h3", { text: t("styles.archivedCount", { count: archived.length }) }));
    for (const sample of archived) drawer.append(styleSampleCard(sample));
  }
  return drawer;
}

function knowledgeForm() {
  const kind = element("select", { name: "kind", attrs: { "aria-label": t("styles.kindLabel") } }, [
    element("option", { value: "fact", text: t("styles.kindFact") }),
    element("option", { value: "constraint", text: t("styles.kindConstraint") }),
  ]);
  const severity = element("select", { name: "severity", disabled: true, attrs: { "aria-label": t("styles.severityLabel") } }, [
    element("option", { value: "hard", text: t("styles.severityHard") }),
    element("option", { value: "soft", text: t("styles.severitySoft") }),
  ]);
  kind.addEventListener("change", () => { severity.disabled = kind.value !== "constraint"; });
  const sensitivity = element("select", { name: "sensitivity", attrs: { "aria-label": t("styles.sensitivityLabel") } }, [
    element("option", { value: "local_sensitive", text: t("styles.sensitivity.local_sensitive") }),
    element("option", { value: "never_send", text: t("styles.sensitivity.never_send") }),
    element("option", { value: "local", text: t("styles.sensitivity.local") }),
    element("option", { value: "public", text: t("styles.sensitivity.public") }),
  ]);
  const content = element("textarea", {
    name: "content",
    placeholder: t("styles.contentPlaceholder"),
    attrs: { required: "", rows: "3", maxlength: "65536" },
  });
  const form = element("form", { className: "knowledge-form" }, [
    element("div", { className: "knowledge-form-grid" }, [kind, severity, sensitivity]),
    labeledInput(t("styles.titleLabel"), "title", t("styles.titlePlaceholder"), true),
    element("label", { className: "field" }, [element("span", { text: t("styles.contentLabel") }), content]),
    element("button", { className: "primary-button", text: t("styles.addCanonical"), type: "submit" }),
  ]);
  form.addEventListener("submit", createKnowledgeItem);
  return form;
}

function knowledgeItemCard(item) {
  const inactive = item.status !== "canonical";
  const kindLabel = item.kind === "fact"
    ? t("styles.kindLabelFact")
    : (item.severity === "hard" ? t("styles.kindLabelHard") : t("styles.kindLabelSoft"));
  const actions = inactive
    ? [button(t("styles.restoreCanonical"), "small-button", () => updateKnowledgeItemStatus(item, "canonical"))]
    : [
        button(t("styles.archive"), "small-button", () => updateKnowledgeItemStatus(item, "archived")),
        button(t("styles.reject"), "small-button", () => updateKnowledgeItemStatus(item, "rejected")),
      ];
  return element("article", { className: `style-card knowledge-card${inactive ? " archived" : ""}` }, [
    element("div", { className: "style-card-heading" }, [
      element("strong", { text: item.title }),
      element("span", {
        className: `policy-pill${item.sensitivity === "never_send" ? " local-only" : ""}`,
        text: t("styles.itemMeta", { kind: kindLabel, sensitivity: item.sensitivity === "never_send" ? t("styles.localOnly") : item.status }),
      }),
    ]),
    element("p", { text: item.content }),
    element("div", { className: "style-card-footer" }, [
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
    setNotice("success", t("styles.canonicalNotice"));
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
    setNotice("success", status === "canonical" ? t("styles.restoredNotice") : t("styles.statusNotice", { status: status === "archived" ? t("styles.statusArchived") : t("styles.statusRejected") }));
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
        text: sample.sensitivity === "never_send" ? t("styles.sensitivityLocal") : t("styles.sensitivitySendable"),
      }),
    ]),
    element("p", { text: sample.content }),
    element("div", { className: "style-card-footer" }, [
      button(archived ? t("styles.reenable") : t("styles.archive"), "small-button", () => updateStyleSampleStatus(
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
      throw { code: "STYLE_SELECTION_REQUIRED", message: t("error.styleSelectionRequired") };
    }
    const content = block.plainText.slice(selection.from, selection.to).trim();
    if (!content) {
      throw { code: "STYLE_SELECTION_REQUIRED", message: t("error.styleSampleBlank") };
    }
    const document = state.workspace.documents.find((item) => item.id === block.documentId);
    const created = await invokeHost("create_style_sample", {
      input: {
        schemaVersion: 1,
        title: t("styles.styleSampleTitle", { title: document?.title ?? t("doc.textIcon"), count: state.styleSamples.length + 1 }),
        content,
        sensitivity: "local_sensitive",
      },
    });
    state.styleSamples = [created, ...state.styleSamples];
    state.stylesOpen = true;
    state.providersOpen = false;
    state.versionsOpen = false;
    state.timelineOpen = false;
    setNotice("success", t("styles.styleSamplePinned"));
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
    setNotice("success", status === "canonical" ? t("styles.styleSampleReenabled") : t("styles.styleSampleArchived"));
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
        element("h2", { text: t("providers.title") }),
      ]),
      iconOnlyButton("close", "icon-button", toggleProviders, { title: t("common.close") }),
    ]),
    element("nav", { className: "provider-tabs", attrs: { "aria-label": t("providers.ariaLabel") } },
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
  if (providerId === "openai_compatible") {
    drawer.append(compatibleProviderPanel());
    return drawer;
  }
  const form = element("form", { className: "provider-form" }, [
    element("div", { className: `credential-status${!credentialRequired || settings.credentialExists ? " connected" : ""}` }, [
      element("span", { className: "status-dot" }),
      element("strong", {
        text: credentialRequired
          ? (settings.credentialExists ? t("providers.credentialStored") : t("providers.credentialMissing"))
          : t("providers.fixedLocalEndpoint"),
      }),
    ]),
    checkboxField(t("providers.enableProvider"), "enabled", settings.enabled),
    ...(providerId === "ollama"
      ? ollamaProviderFields(settings)
      : [labeledInput(t("providers.defaultModel"), "defaultModel", PROVIDER_PRESETS[providerId].defaultModel, true, settings.defaultModel)]),
    ...(providerId === "qwen" ? qwenProviderFields(settings) : []),
    ...(credentialRequired ? [
      passwordField(t("providers.apiKey"), "apiKey", settings.credentialExists ? t("providers.apiKeyKeepEmpty") : t("providers.apiKeySendToHost")),
      element("p", {
        className: "form-hint",
        text: t("providers.apiKeyHint"),
      }),
    ] : [
      element("p", {
        className: "form-hint",
        text: t("providers.ollamaHint"),
      }),
    ]),
    element("button", { className: "primary-button", text: t("providers.saveSettings"), type: "submit" }),
    credentialRequired && settings.credentialExists
      ? button(t("providers.deleteCredential"), "quiet-button provider-delete", deleteProviderCredential)
      : null,
  ]);
  form.addEventListener("submit", saveProviderSettings);
  drawer.append(form);
  return drawer;
}

function compatibleProviderPanel() {
  const settings = state.providerSettings.openai_compatible;
  const endpoint = settings.trustedEndpoint;
  const endpointSelect = element("select", { name: "trustedEndpointId" }, [
    element("option", { value: "", text: t("providers.trustedEndpointPlaceholder") }),
    ...state.trustedModelEndpoints.map((candidate) => element("option", {
      value: candidate.id,
      text: `${candidate.label} · ${candidate.baseUrl}`,
    })),
  ]);
  endpointSelect.value = endpoint?.id ?? "";
  endpointSelect.addEventListener("change", async () => {
    const trustedEndpoint = state.trustedModelEndpoints.find(
      (candidate) => candidate.id === endpointSelect.value,
    ) ?? null;
    state.providerSettings.openai_compatible = {
      ...state.providerSettings.openai_compatible,
      trustedEndpoint,
      trustedEndpointId: trustedEndpoint?.id ?? "",
      enabled: false,
      credentialExists: false,
    };
    state.compatibleDiscovery = null;
    persistProviderSettings();
    await refreshProviderSecretStatus();
    renderWorkspace();
  });
  const panel = element("div", { className: "provider-compatible-panel" }, [
    element("label", { className: "field" }, [
      element("span", { text: t("providers.trustedEndpoints") }),
      endpointSelect,
    ]),
  ]);
  if (endpoint) panel.append(compatibleEndpointSettingsForm(settings, endpoint));
  else panel.append(element("p", {
    className: "form-hint",
    text: t("providers.registerHint"),
  }));
  panel.append(compatibleEndpointRegistrationForm());
  return panel;
}

function compatibleEndpointSettingsForm(settings, endpoint) {
  const discovery = state.compatibleDiscovery;
  const models = discovery?.endpointId === endpoint.id && discovery.state === "success"
    ? discovery.models
    : [];
  const modelInput = element("input", {
    name: "defaultModel",
    value: settings.defaultModel,
    placeholder: t("providers.modelIdPlaceholder"),
    attrs: { list: "compatible-model-list", autocomplete: "off" },
  });
  const status = discovery?.endpointId !== endpoint.id
    ? t("providers.discoverIdle")
    : discovery.state === "loading"
      ? t("providers.discoverLoading")
      : discovery.state === "success"
        ? t("providers.discoverDone", { count: models.length })
        : discovery.message;
  const form = element("form", { className: "provider-form compatible-settings-form" }, [
    element("div", { className: `credential-status${settings.credentialExists ? " connected" : ""}` }, [
      element("span", { className: "status-dot" }),
      element("strong", {
        text: settings.credentialExists ? t("providers.endpointCredentialStored") : t("providers.endpointCredentialMissing"),
      }),
    ]),
    element("p", { className: "form-hint" }, [
      element("span", { text: t("providers.actualTarget") }),
      element("code", { text: `${endpoint.baseUrl}/chat/completions` }),
    ]),
    checkboxField(t("providers.enableEndpoint"), "enabled", settings.enabled),
    element("label", { className: "field" }, [element("span", { text: t("providers.defaultModel") }), modelInput]),
    element("datalist", { attrs: { id: "compatible-model-list" } },
      models.map((model) => element("option", { value: model.id }))),
    passwordField(t("providers.apiKey"), "apiKey", settings.credentialExists ? t("providers.apiKeyKeepEmpty") : t("providers.apiKeyWriteOnly")),
    button(
      discovery?.endpointId === endpoint.id && discovery.state === "loading"
        ? t("providers.probing")
        : t("providers.probeModels"),
      "quiet-button",
      probeCompatibleModels,
      { disabled: discovery?.endpointId === endpoint.id && discovery.state === "loading" },
    ),
    element("p", { className: "form-hint", text: status, attrs: { role: "status" } }),
    element("button", { className: "primary-button", text: t("providers.saveEndpoint"), type: "submit" }),
    settings.credentialExists
      ? button(t("providers.deleteCredential"), "quiet-button provider-delete", deleteProviderCredential)
      : null,
    button(t("providers.removeTrustedEndpoint"), "quiet-button provider-delete", removeCompatibleEndpoint),
  ]);
  form.addEventListener("submit", saveCompatibleProviderSettings);
  return form;
}

function compatibleEndpointRegistrationForm() {
  const maxField = element("select", { name: "maxOutputTokenField" }, [
    element("option", { value: "max_tokens", text: t("providers.maxTokensDefault") }),
    element("option", { value: "max_completion_tokens", text: "max_completion_tokens" }),
  ]);
  const form = element("form", { className: "provider-form compatible-register-form" }, [
    element("h3", { text: t("providers.registerNew") }),
    labeledInput(t("providers.nameLabel"), "label", t("providers.namePlaceholder"), true, ""),
    labeledInput(t("providers.hostnameLabel"), "hostname", t("providers.hostnamePlaceholder"), true, ""),
    labeledInput("Base path", "basePath", "/v1", true, "/v1"),
    labeledInput(t("providers.confirmOriginLabel"), "confirmedOrigin", "https:" + "//api.vendor.com", true, ""),
    checkboxField(t("providers.jsonObjectOption"), "jsonObject", false),
    checkboxField(t("providers.streamUsageOption"), "streamUsage", false),
    element("label", { className: "field" }, [
      element("span", { text: t("providers.outputTokenField") }),
      maxField,
    ]),
    element("p", {
      className: "form-hint",
      text: t("providers.trustConfirmHint"),
    }),
    element("button", { className: "primary-button", text: t("providers.trustSubmit"), type: "submit" }),
  ]);
  form.addEventListener("submit", registerCompatibleEndpoint);
  return form;
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
    ["china", t("providers.region.china")],
    ["singapore", t("providers.region.singapore")],
    ["us", t("providers.region.us")],
    ["germany", t("providers.region.germany")],
    ["japan", t("providers.region.japan")],
  ].map(([value, label]) => element("option", { value, text: label })));
  select.value = settings.qwenRegion;
  return [
    element("label", { className: "field" }, [element("span", { text: t("providers.regionLabel") }), select]),
    labeledInput(t("providers.workspaceIdLabel"), "qwenWorkspaceId", t("providers.workspaceIdHint"), false, settings.qwenWorkspaceId),
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
    ? t("providers.ollama.connecting")
    : discovery?.state === "success"
      ? (models.length > 0 ? t("providers.ollama.discovered", { count: models.length }) : t("providers.ollama.empty"))
      : discovery?.state === "error"
        ? discovery.message
        : t("providers.ollama.idle");
  return [
    element("label", { className: "field" }, [element("span", { text: t("providers.defaultModel") }), input]),
    element("datalist", { attrs: { id: "ollama-model-list" } },
      models.map((model) => element("option", { value: model.id }))),
    button(
      discovery?.state === "loading" ? t("providers.ollama.detecting") : t("providers.ollama.detect"),
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
      message: t("providers.ollama.unavailable", { message: normalizeHostError(error).message }),
    };
  }
  renderWorkspace();
}

async function registerCompatibleEndpoint(event) {
  event.preventDefault();
  const form = event.currentTarget;
  const input = {
    schemaVersion: 1,
    label: formValue(form, "label"),
    hostname: formValue(form, "hostname"),
    basePath: formValue(form, "basePath"),
    confirmedOrigin: formValue(form, "confirmedOrigin"),
    capabilities: {
      jsonObject: form.elements.namedItem("jsonObject").checked,
      streamUsage: form.elements.namedItem("streamUsage").checked,
      maxOutputTokenField: formValue(form, "maxOutputTokenField"),
    },
  };
  setFormBusy(form, true);
  try {
    const endpoint = await invokeHost("register_trusted_model_endpoint", {
      input,
    });
    await refreshTrustedModelEndpoints();
    state.providerSettings.openai_compatible = {
      ...state.providerSettings.openai_compatible,
      trustedEndpointId: endpoint.id,
      trustedEndpoint: state.trustedModelEndpoints.find((item) => item.id === endpoint.id) ?? null,
      enabled: false,
      credentialExists: false,
    };
    persistProviderSettings();
    setNotice("success", t("providers.endpointRegistered", { label: endpoint.label, baseUrl: endpoint.baseUrl }));
  } catch (error) {
    setNotice("error", normalizeHostError(error).message);
  } finally {
    setFormBusy(form, false);
    renderWorkspace();
  }
}

async function saveCompatibleProviderSettings(event) {
  event.preventDefault();
  const form = event.currentTarget;
  const previous = state.providerSettings.openai_compatible;
  const endpoint = previous.trustedEndpoint;
  if (!endpoint) return;
  const apiKey = formValue(form, "apiKey");
  const enabled = form.elements.namedItem("enabled").checked;
  const defaultModel = formValue(form, "defaultModel");
  setFormBusy(form, true);
  try {
    if (!defaultModel) {
      throw { code: "INVALID_MODEL_REQUEST", message: t("error.modelIdRequired") };
    }
    if (enabled && !apiKey && !previous.credentialExists) {
      throw { code: "PROVIDER_CREDENTIAL_MISSING", message: t("error.endpointCredentialMissing") };
    }
    if (apiKey) {
      await invokeHost("store_provider_secret", {
        reference: credentialReference("openai_compatible", endpoint),
        secret: apiKey,
      });
    }
    state.providerSettings.openai_compatible = {
      ...previous,
      enabled,
      defaultModel,
      credentialExists: apiKey ? true : previous.credentialExists,
    };
    persistProviderSettings();
    await refreshProviderSecretStatus();
    setNotice("success", t("providers.endpointSaved", { label: endpoint.label }));
  } catch (error) {
    setNotice("error", normalizeHostError(error).message);
  } finally {
    setFormBusy(form, false);
    renderWorkspace();
  }
}

async function probeCompatibleModels(event) {
  const endpoint = state.providerSettings.openai_compatible.trustedEndpoint;
  if (!endpoint) return;
  const form = event.currentTarget.closest("form");
  state.providerSettings.openai_compatible = {
    ...state.providerSettings.openai_compatible,
    defaultModel: formValue(form, "defaultModel"),
  };
  state.compatibleDiscovery = { state: "loading", endpointId: endpoint.id, models: [] };
  renderWorkspace();
  try {
    const response = await invokeHost("list_openai_compatible_models", {
      input: { schemaVersion: 1, endpointId: endpoint.id },
    });
    if (response.endpointId !== endpoint.id) {
      throw { code: "PROVIDER_PROTOCOL", message: t("error.providerProtocol") };
    }
    const models = Array.isArray(response.models)
      ? response.models.filter((model) => model && typeof model.id === "string")
      : [];
    state.compatibleDiscovery = {
      state: "success",
      endpointId: endpoint.id,
      models,
    };
  } catch (error) {
    state.compatibleDiscovery = {
      state: "error",
      endpointId: endpoint.id,
      models: [],
      message: t("providers.probeFailed", { message: normalizeHostError(error).message }),
    };
  }
  renderWorkspace();
}

async function removeCompatibleEndpoint() {
  const endpoint = state.providerSettings.openai_compatible.trustedEndpoint;
  if (!endpoint) return;
  if (!window.confirm(t("providers.removeConfirm", { baseUrl: endpoint.baseUrl }))) return;
  try {
    await invokeHost("remove_trusted_model_endpoint", {
      input: { schemaVersion: 1, endpointId: endpoint.id },
    });
    state.providerSettings.openai_compatible = {
      ...state.providerSettings.openai_compatible,
      enabled: false,
      trustedEndpoint: null,
      trustedEndpointId: "",
      credentialExists: false,
    };
    state.compatibleDiscovery = null;
    await refreshTrustedModelEndpoints();
    persistProviderSettings();
    setNotice("success", t("providers.endpointRemoved", { label: endpoint.label }));
  } catch (error) {
    setNotice("error", normalizeHostError(error).message);
  }
  renderWorkspace();
}

async function saveProviderSettings(event) {
  event.preventDefault();
  const form = event.currentTarget;
  const providerId = state.selectedProviderId;
  const previous = state.providerSettings[providerId];
  const credentialRequired = providerRequiresCredential(providerId);
  const apiKey = credentialRequired ? formValue(form, "apiKey") : "";
  const enabled = form.elements.namedItem("enabled").checked;
  const defaultModel = formValue(form, "defaultModel");
  const qwenRegion = providerId === "qwen" ? formValue(form, "qwenRegion") : "china";
  const qwenWorkspaceId = providerId === "qwen" ? formValue(form, "qwenWorkspaceId") : "";
  setFormBusy(form, true);
  try {
    if (apiKey) {
      await invokeHost("store_provider_secret", {
        reference: credentialReference(providerId),
        secret: apiKey,
      });
    }
    if (enabled && credentialRequired && !apiKey && !previous.credentialExists) {
      throw { code: "PROVIDER_CREDENTIAL_MISSING", message: t("error.providerCredentialMissing") };
    }
    state.providerSettings[providerId] = {
      ...previous,
      enabled,
      defaultModel,
      qwenRegion,
      qwenWorkspaceId,
      credentialExists: apiKey ? true : previous.credentialExists,
    };
    persistProviderSettings();
    await refreshProviderSecretStatus();
    setNotice(
      "success",
      credentialRequired
        ? t("providers.providerSavedCredential", { label: PROVIDER_PRESETS[providerId].label })
        : t("providers.providerSavedLocal", { label: PROVIDER_PRESETS[providerId].label }),
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
    const endpoint = providerId === "openai_compatible"
      ? state.providerSettings.openai_compatible.trustedEndpoint
      : null;
    await invokeHost("delete_provider_secret", {
      reference: credentialReference(providerId, endpoint),
    });
    state.providerSettings[providerId] = {
      ...state.providerSettings[providerId],
      enabled: false,
      credentialExists: false,
    };
    persistProviderSettings();
    setNotice("success", t("providers.credentialDeleted", { label: PROVIDER_PRESETS[providerId].label }));
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
      element("div", {}, [element("span", { className: "eyebrow", text: "VERSION GRAPH" }), element("h2", { text: t("versions.title") })]),
      button("×", "icon-button", toggleVersions, { title: t("common.close") }),
    ]),
  ]);
  if (!history) {
    drawer.append(element("p", { className: "drawer-empty", text: t("versions.loading") }));
    return drawer;
  }
  drawer.append(element("h3", { text: t("versions.checkpoints", { count: history.checkpoints.length }) }));
  if (!history.checkpoints.length) drawer.append(element("p", { className: "drawer-empty", text: t("versions.noCheckpoint") }));
  for (const checkpoint of history.checkpoints) {
    drawer.append(element("article", { className: "version-card" }, [
      element("strong", { text: formatDate(checkpoint.createdAt), title: checkpoint.commitId }),
      button(t("versions.restoreHere"), "small-button", () => restoreCheckpoint(checkpoint.id)),
    ]));
  }
  drawer.append(element("h3", { text: t("versions.commits", { count: history.commits.length }) }));
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
  if (!window.confirm(t("checkpoint.restoreConfirm"))) return;
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
    state.compareResult = null;
    state.conflictDraft = null;
    setNotice("success", t("checkpoint.restoredBlocks", { count: response.changedBlocks }));
    scheduleSummaryRefresh();
    await loadVersionHistory();
  } catch (error) {
    setNotice("error", normalizeHostError(error).message);
    renderWorkspace();
  }
}

function toggleBackupWizard() {
  state.backupWizardOpen = !state.backupWizardOpen;
  if (state.backupWizardOpen) {
    state.providersOpen = false;
    state.stylesOpen = false;
    state.candidatesOpen = false;
    state.versionsOpen = false;
    state.insightsOpen = false;
    state.timelineOpen = false;
    state.compareOpen = false;
    state.backupWizardStep = 1;
    state.backupWizardMode = null;
    state.backupWizardResult = null;
    state.backupWizardManifest = null;
    state.backupWizardWarnings = [];
  }
  renderWorkspace();
}

function resetBackupWizardForm() {
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

function browseBackupOutputPath(docFactory) {
  const picker = element("input", {
    type: "file",
    attrs: {
      "aria-label": t("wizard.backup.outputPath"),
      webkitdirectory: "",
      directory: "",
    },
  }, [], docFactory);
  picker.hidden = true;
  picker.addEventListener("change", () => {
    const file = picker.files?.[0];
    picker.remove();
    if (!file) return;
    const dir = file.path || "";
    if (dir) state.backupWizardForm.outputPath = dir;
    renderWorkspace();
  }, { once: true });
  (docFactory.body || docFactory).append(picker);
  picker.click();
}

function browseBackupArchivePath(docFactory) {
  const picker = element("input", {
    type: "file",
    attrs: {
      accept: ".optimizer-backup,.zip",
      "aria-label": t("wizard.restore.archivePath"),
    },
  }, [], docFactory);
  picker.hidden = true;
  picker.addEventListener("change", () => {
    const file = picker.files?.[0];
    picker.remove();
    if (!file) return;
    state.backupWizardForm.archivePath = file.path || file.name || "";
    renderWorkspace();
  }, { once: true });
  (docFactory.body || docFactory).append(picker);
  picker.click();
}

function browseRestoreTargetDirectory(docFactory) {
  const picker = element("input", {
    type: "file",
    attrs: {
      "aria-label": t("wizard.restore.targetDirectory"),
      webkitdirectory: "",
      directory: "",
    },
  }, [], docFactory);
  picker.hidden = true;
  picker.addEventListener("change", () => {
    const file = picker.files?.[0];
    picker.remove();
    if (!file) return;
    const dir = file.path || "";
    if (dir) state.backupWizardForm.targetDirectory = dir;
    renderWorkspace();
  }, { once: true });
  (docFactory.body || docFactory).append(picker);
  picker.click();
}

function backupWizardStep1(docFactory) {
  const backupRadio = element("input", {
    type: "radio",
    name: "wizard-mode",
    value: "backup",
    attrs: { checked: state.backupWizardMode === "backup" ? "" : undefined },
  }, [], docFactory);
  backupRadio.addEventListener("change", () => {
    state.backupWizardMode = "backup";
  });
  const restoreRadio = element("input", {
    type: "radio",
    name: "wizard-mode",
    value: "restore",
    attrs: { checked: state.backupWizardMode === "restore" ? "" : undefined },
  }, [], docFactory);
  restoreRadio.addEventListener("change", () => {
    state.backupWizardMode = "restore";
  });
  return element("div", { className: "wizard-step-content" }, [
    element("div", { className: "wizard-options" }, [
      element("label", { className: "wizard-option-card" }, [
        backupRadio,
        element("div", { className: "wizard-option-body" }, [
          element("strong", { text: t("wizard.operation.backup") }, [], docFactory),
          element("span", { className: "wizard-option-desc", text: t("wizard.operation.backupDescription") }, [], docFactory),
        ], docFactory),
      ], docFactory),
      element("label", { className: "wizard-option-card" }, [
        restoreRadio,
        element("div", { className: "wizard-option-body" }, [
          element("strong", { text: t("wizard.operation.restore") }, [], docFactory),
          element("span", { className: "wizard-option-desc", text: t("wizard.operation.restoreDescription") }, [], docFactory),
        ], docFactory),
      ], docFactory),
    ], docFactory),
    element("div", { className: "wizard-actions" }, [
      button(t("wizard.cancel"), "quiet-button", toggleBackupWizard, {}, docFactory),
      button(
        t("wizard.next"),
        "small-button",
        () => {
          if (!state.backupWizardMode) {
            const checked = docFactory.querySelector?.("input[name='wizard-mode']:checked");
            if (checked) state.backupWizardMode = checked.value;
          }
          if (!state.backupWizardMode) return;
          state.backupWizardStep = 2;
          renderWorkspace();
        },
        {}, docFactory,
      ),
    ], docFactory),
  ], docFactory);
}

function backupWizardStep2Backup(docFactory) {
  const form = element("form", { className: "wizard-form", attrs: { "data-wizard-form": "backup" } }, [
    element("label", { className: "field" }, [
      element("span", { className: "field-label", text: t("wizard.backup.outputPath") }, [], docFactory),
      element("div", { className: "field-row" }, [
        element("input", {
          type: "text",
          name: "outputPath",
          placeholder: t("wizard.backup.outputPathPlaceholder"),
          value: state.backupWizardForm.outputPath,
          attrs: { "aria-label": t("wizard.backup.outputPath") },
        }, [], docFactory),
        button(t("wizard.backup.browse"), "small-button", (event) => {
          event.preventDefault();
          browseBackupOutputPath(docFactory);
        }, {}, docFactory),
      ], docFactory),
    ], docFactory),
    element("label", { className: "field checkbox-field" }, (() => {
      const cb = element("input", { type: "checkbox", name: "includeEndpoints" }, [], docFactory);
      cb.checked = state.backupWizardForm.includeEndpoints;
      return [cb, element("span", { className: "field-label", text: t("wizard.backup.includeEndpoints") }, [], docFactory)];
    })()),
    element("p", { className: "field-hint", text: t("wizard.backup.includeEndpointsHint") }, [], docFactory),
    element("label", { className: "field checkbox-field" }, (() => {
      const cb = element("input", { type: "checkbox", name: "includeRecent" }, [], docFactory);
      cb.checked = state.backupWizardForm.includeRecent;
      return [cb, element("span", { className: "field-label", text: t("wizard.backup.includeRecent") }, [], docFactory)];
    })()),
  ], docFactory);

  form.addEventListener("change", () => {
    state.backupWizardForm.outputPath = form.elements.outputPath.value || "";
    state.backupWizardForm.includeEndpoints = form.elements.includeEndpoints.checked;
    state.backupWizardForm.includeRecent = form.elements.includeRecent.checked;
  });

  return element("div", { className: "wizard-step-content" }, [
    form,
    element("div", { className: "wizard-actions" }, [
      button(t("wizard.back"), "quiet-button", () => {
        state.backupWizardStep = 1;
        renderWorkspace();
      }, {}, docFactory),
      button(
        t("wizard.next"),
        "small-button",
        () => {
          if (!state.backupWizardForm.outputPath.trim()) {
            setNotice("error", t("wizard.error.noOutputPath"));
            renderWorkspace();
            return;
          }
          state.backupWizardWarnings = [t("wizard.warning.noSecret")];
          state.backupWizardStep = 3;
          renderWorkspace();
        },
        {}, docFactory,
      ),
    ], docFactory),
  ], docFactory);
}

function backupWizardStep2Restore(docFactory) {
  const form = element("form", { className: "wizard-form", attrs: { "data-wizard-form": "restore" } }, [
    element("label", { className: "field" }, [
      element("span", { className: "field-label", text: t("wizard.restore.archivePath") }, [], docFactory),
      element("div", { className: "field-row" }, [
        element("input", {
          type: "text",
          name: "archivePath",
          placeholder: t("wizard.restore.archivePathPlaceholder"),
          value: state.backupWizardForm.archivePath,
          attrs: { "aria-label": t("wizard.restore.archivePath") },
        }, [], docFactory),
        button(t("wizard.restore.browse"), "small-button", (event) => {
          event.preventDefault();
          browseBackupArchivePath(docFactory);
        }, {}, docFactory),
      ], docFactory),
    ], docFactory),
    element("label", { className: "field" }, [
      element("span", { className: "field-label", text: t("wizard.restore.targetDirectory") }, [], docFactory),
      element("div", { className: "field-row" }, [
        element("input", {
          type: "text",
          name: "targetDirectory",
          placeholder: t("wizard.restore.targetDirectoryPlaceholder"),
          value: state.backupWizardForm.targetDirectory,
          attrs: { "aria-label": t("wizard.restore.targetDirectory") },
        }, [], docFactory),
        button(t("wizard.restore.browse"), "small-button", (event) => {
          event.preventDefault();
          browseRestoreTargetDirectory(docFactory);
        }, {}, docFactory),
      ], docFactory),
    ], docFactory),
    element("label", { className: "field" }, [
      element("span", { className: "field-label", text: t("wizard.restore.newProjectId") }, [], docFactory),
      element("input", {
        type: "text",
        name: "newProjectId",
        placeholder: t("wizard.restore.newProjectIdPlaceholder"),
        value: state.backupWizardForm.newProjectId,
        attrs: { "aria-label": t("wizard.restore.newProjectId") },
      }, [], docFactory),
    ], docFactory),
    element("label", { className: "field checkbox-field" }, (() => {
      const cb = element("input", { type: "checkbox", name: "overwrite" }, [], docFactory);
      cb.checked = state.backupWizardForm.overwrite;
      return [cb, element("span", { className: "field-label", text: t("wizard.restore.overwrite") }, [], docFactory)];
    })()),
  ], docFactory);

  form.addEventListener("change", () => {
    state.backupWizardForm.archivePath = form.elements.archivePath.value || "";
    state.backupWizardForm.targetDirectory = form.elements.targetDirectory.value || "";
    state.backupWizardForm.newProjectId = form.elements.newProjectId.value || "";
    state.backupWizardForm.overwrite = form.elements.overwrite.checked;
  });

  return element("div", { className: "wizard-step-content" }, [
    form,
    element("div", { className: "wizard-actions" }, [
      button(t("wizard.back"), "quiet-button", () => {
        state.backupWizardStep = 1;
        renderWorkspace();
      }, {}, docFactory),
      button(
        t("wizard.next"),
        "small-button",
        () => {
          if (!state.backupWizardForm.archivePath.trim()) {
            setNotice("error", t("wizard.error.noArchivePath"));
            renderWorkspace();
            return;
          }
          if (!state.backupWizardForm.targetDirectory.trim()) {
            setNotice("error", t("wizard.error.noTargetDirectory"));
            renderWorkspace();
            return;
          }
          state.backupWizardWarnings = [
            t("wizard.warning.noSecret"),
            t("wizard.warning.newProjectId"),
          ];
          if (state.backupWizardForm.overwrite) {
            state.backupWizardWarnings.push(t("wizard.warning.overwrite"));
          }
          state.backupWizardStep = 3;
          renderWorkspace();
        },
        {}, docFactory,
      ),
    ], docFactory),
  ], docFactory);
}

function backupWizardPreviewRows(manifest, docFactory) {
  if (!manifest) return [];
  return [
    [t("wizard.preview.manifestSchemaVersion"), String(manifest.schemaVersion ?? "")],
    [t("wizard.preview.sourceProjectId"), manifest.sourceProjectId || ""],
    [t("wizard.preview.sourcePath"), manifest.sourcePath || ""],
    [t("wizard.preview.generatedAt"), manifest.generatedAt || ""],
    [t("wizard.preview.includedItems"), (manifest.includedItems || []).join(", ")],
    [t("wizard.preview.optimizerVersion"), manifest.optimizerVersion || ""],
    [t("wizard.preview.schemaDbVersion"), String(manifest.schemaDbVersion ?? "")],
  ].map(([label, value]) =>
    element("div", { className: "preview-row" }, [
      element("span", { className: "preview-label", text: label }, [], docFactory),
      element("span", { className: "preview-value", text: value }, [], docFactory),
    ], docFactory),
  );
}

function backupWizardStep3Preview(docFactory) {
  const isBackup = state.backupWizardMode === "backup";
  const warnings = state.backupWizardWarnings.map((warning) =>
    element("p", { className: "wizard-warning", text: warning }, [], docFactory),
  );

  if (isBackup) {
    return element("div", { className: "wizard-step-content" }, [
      element("div", { className: "wizard-preview" }, [
        element("h3", { text: t("wizard.preview.title") }, [], docFactory),
        element("div", { className: "preview-rows" }, [
          element("div", { className: "preview-row" }, [
            element("span", { className: "preview-label", text: t("wizard.backup.outputPath") }, [], docFactory),
            element("span", { className: "preview-value", text: state.backupWizardForm.outputPath }, [], docFactory),
          ], docFactory),
          element("div", { className: "preview-row" }, [
            element("span", { className: "preview-label", text: t("wizard.backup.includeEndpoints") }, [], docFactory),
            element("span", { className: "preview-value", text: state.backupWizardForm.includeEndpoints ? "yes" : "no" }, [], docFactory),
          ], docFactory),
          element("div", { className: "preview-row" }, [
            element("span", { className: "preview-label", text: t("wizard.backup.includeRecent") }, [], docFactory),
            element("span", { className: "preview-value", text: state.backupWizardForm.includeRecent ? "yes" : "no" }, [], docFactory),
          ], docFactory),
        ], docFactory),
      ], docFactory),
      element("div", { className: "wizard-warnings" }, warnings),
      element("div", { className: "wizard-actions" }, [
        button(t("wizard.back"), "quiet-button", () => {
          state.backupWizardStep = 2;
          renderWorkspace();
        }, {}, docFactory),
        button(
          state.backupWizardBusy ? t("common.loading") : t("wizard.backup.execute"),
          "small-button",
          executeBackup,
          { disabled: state.backupWizardBusy },
          docFactory,
        ),
      ], docFactory),
    ], docFactory);
  }

  return element("div", { className: "wizard-step-content" }, [
    element("div", { className: "wizard-preview" }, [
      element("h3", { text: t("wizard.preview.title") }, [], docFactory),
      element("div", { className: "preview-rows" }, [
        element("div", { className: "preview-row" }, [
          element("span", { className: "preview-label", text: t("wizard.restore.archivePath") }, [], docFactory),
          element("span", { className: "preview-value", text: state.backupWizardForm.archivePath }, [], docFactory),
        ], docFactory),
        element("div", { className: "preview-row" }, [
          element("span", { className: "preview-label", text: t("wizard.restore.targetDirectory") }, [], docFactory),
          element("span", { className: "preview-value", text: state.backupWizardForm.targetDirectory }, [], docFactory),
        ], docFactory),
        element("div", { className: "preview-row" }, [
          element("span", { className: "preview-label", text: t("wizard.restore.newProjectId") }, [], docFactory),
          element("span", { className: "preview-value", text: state.backupWizardForm.newProjectId || t("wizard.restore.newProjectIdPlaceholder") }, [], docFactory),
        ], docFactory),
        element("div", { className: "preview-row" }, [
          element("span", { className: "preview-label", text: t("wizard.restore.overwrite") }, [], docFactory),
          element("span", { className: "preview-value", text: state.backupWizardForm.overwrite ? "yes" : "no" }, [], docFactory),
        ], docFactory),
      ], docFactory),
    ], docFactory),
    element("div", { className: "wizard-warnings" }, warnings),
    element("div", { className: "wizard-actions" }, [
      button(t("wizard.back"), "quiet-button", () => {
        state.backupWizardStep = 2;
        renderWorkspace();
      }, {}, docFactory),
      button(
        state.backupWizardBusy ? t("common.loading") : t("wizard.restore.execute"),
        "small-button",
        executeRestore,
        { disabled: state.backupWizardBusy },
        docFactory,
      ),
    ], docFactory),
  ], docFactory);
}

function backupWizardResultView(docFactory) {
  const result = state.backupWizardResult;
  const manifest = result.manifest || state.backupWizardManifest;
  const isBackup = state.backupWizardMode === "backup";
  const rows = backupWizardPreviewRows(manifest, docFactory);

  if (isBackup) {
    rows.unshift(
      element("div", { className: "preview-row" }, [
        element("span", { className: "preview-label", text: t("wizard.preview.bytesWritten") }, [], docFactory),
        element("span", { className: "preview-value", text: String(result.bytesWritten ?? "") }, [], docFactory),
      ], docFactory),
      element("div", { className: "preview-row" }, [
        element("span", { className: "preview-label", text: t("wizard.preview.itemCount") }, [], docFactory),
        element("span", { className: "preview-value", text: String(result.itemCount ?? "") }, [], docFactory),
      ], docFactory),
    );
  } else {
    rows.unshift(
      element("div", { className: "preview-row" }, [
        element("span", { className: "preview-label", text: t("wizard.preview.projectId") }, [], docFactory),
        element("span", { className: "preview-value", text: result.projectId || "" }, [], docFactory),
      ], docFactory),
      element("div", { className: "preview-row" }, [
        element("span", { className: "preview-label", text: t("wizard.preview.projectPath") }, [], docFactory),
        element("span", { className: "preview-value", text: result.projectPath || "" }, [], docFactory),
      ], docFactory),
      element("div", { className: "preview-row" }, [
        element("span", { className: "preview-label", text: t("wizard.preview.restoredItems") }, [], docFactory),
        element("span", { className: "preview-value", text: (result.restoredItems || []).join(", ") }, [], docFactory),
      ], docFactory),
    );
  }

  return element("div", { className: "wizard-step-content" }, [
    element("p", {
      className: "wizard-success",
      text: isBackup
        ? t("wizard.success.backup", { path: result.archivePath || "" })
        : t("wizard.success.restore", { path: result.projectPath || "" }),
    }, [], docFactory),
    element("div", { className: "wizard-preview" }, [
      element("h3", { text: t("wizard.preview.title") }, [], docFactory),
      element("div", { className: "preview-rows" }, rows),
    ], docFactory),
    ...(result.warnings || []).map((warning) =>
      element("p", { className: "wizard-warning", text: warning }, [], docFactory),
    ),
    element("div", { className: "wizard-actions" }, [
      button(t("wizard.close"), "small-button", () => {
        state.backupWizardOpen = false;
        state.backupWizardResult = null;
        state.backupWizardManifest = null;
        resetBackupWizardForm();
        renderWorkspace();
      }, {}, docFactory),
    ], docFactory),
  ], docFactory);
}

function backupRestoreWizard(docFactory = (typeof document !== "undefined" ? document : null)) {
  const drawer = element("aside", { className: "version-drawer backup-wizard-drawer", attrs: { role: "dialog", "aria-modal": "true", "aria-labelledby": "backup-wizard-title" } }, [
    element("div", { className: "drawer-heading" }, [
      element("div", {}, [
        element("span", { className: "eyebrow", text: "BACKUP & RESTORE" }, [], docFactory),
        element("h2", { text: t("wizard.title"), attrs: { id: "backup-wizard-title" } }, [], docFactory),
      ], docFactory),
      button("×", "icon-button", toggleBackupWizard, { title: t("wizard.close"), attrs: { "aria-label": t("a11y.closeDrawer") } }, docFactory),
    ], docFactory),
  ], docFactory);
  attachDrawerKeyboard(drawer, docFactory, () => { state.backupWizardOpen = false; });

  const stepLabels = [t("wizard.step.select"), t("wizard.step.path"), t("wizard.step.preview")];
  const steps = element("ol", { className: "wizard-steps" },
    stepLabels.map((label, index) =>
      element("li", {
        className: `wizard-step${state.backupWizardStep >= index + 1 ? " active" : ""}`,
        text: label,
      }, [], docFactory),
    ),
    docFactory,
  );
  drawer.append(steps);

  if (state.backupWizardResult) {
    drawer.append(backupWizardResultView(docFactory));
    return drawer;
  }

  if (state.backupWizardStep === 1) {
    drawer.append(backupWizardStep1(docFactory));
  } else if (state.backupWizardStep === 2) {
    drawer.append(
      state.backupWizardMode === "backup"
        ? backupWizardStep2Backup(docFactory)
        : backupWizardStep2Restore(docFactory),
    );
  } else if (state.backupWizardStep === 3) {
    drawer.append(backupWizardStep3Preview(docFactory));
  }

  return drawer;
}

async function executeBackup() {
  state.backupWizardBusy = true;
  renderWorkspace();
  try {
    await flushAll();
    const result = await invokeHost("export_project_backup", {
      input: {
        schemaVersion: 1,
        includeEndpoints: state.backupWizardForm.includeEndpoints,
        includeRecent: state.backupWizardForm.includeRecent,
        outputPath: state.backupWizardForm.outputPath,
      },
    });
    state.backupWizardResult = result;
    state.backupWizardManifest = result.manifest || null;
    setNotice("success", t("wizard.success.backup", { path: result.archivePath || "" }));
  } catch (error) {
    setNotice("error", normalizeHostError(error).message);
  } finally {
    state.backupWizardBusy = false;
    renderWorkspace();
  }
}

async function executeRestore() {
  state.backupWizardBusy = true;
  renderWorkspace();
  try {
    const result = await invokeHost("import_project_backup", {
      input: {
        schemaVersion: 1,
        archivePath: state.backupWizardForm.archivePath,
        targetDirectory: state.backupWizardForm.targetDirectory,
        newProjectId: state.backupWizardForm.newProjectId || null,
        overwrite: state.backupWizardForm.overwrite,
      },
    });
    state.backupWizardResult = result;
    state.backupWizardManifest = result.manifest || null;
    setNotice("success", t("wizard.success.restore", { path: result.projectPath || "" }));
  } catch (error) {
    setNotice("error", normalizeHostError(error).message);
  } finally {
    state.backupWizardBusy = false;
    renderWorkspace();
  }
}

async function closeProject() {
  try {
    clearTimeout(state.summaryRefreshTimer);
    state.summaryRefreshTimer = null;
    state.aiRunning?.controller.abort();
    await flushAll({ allowConflict: true });
    if (state.conflictDraft && !window.confirm(t("save.conflictDraftClose"))) return;
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
    state.aiRetry = null;
    state.aiContextMenu = null;
    state.commandPaletteOpen = false;
    state.insightsOpen = false;
    state.insightsData = null;
    state.insightsBusy = null;
    state.compareOpen = false;
    state.compareBusy = false;
    state.compareResult = null;
    state.timelineOpen = false;
    state.timelineEvents = null;
    state.timelineBusy = false;
    state.timelineSelected = null;
    state.compareSelection = {
      snapshotIdA: "",
      snapshotIdB: "",
      documentIdA: "",
      documentIdB: "",
    };
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

export {
  aiReviewView,
  announceReviewState,
  backupRestoreWizard,
  commandPaletteModal,
  compareBlockRow,
  compareDrawer,
  compareResultView,
  element,
  fpsMonitor,
  insightsDrawer,
  state,
  timelineDrawer,
  TIMELINE_STATE_COLORS,
  toggleBackupWizard,
  virtualizeDiffRows,
};
