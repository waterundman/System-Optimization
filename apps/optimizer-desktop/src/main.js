// v0.9.0 Stage 1: entry aggregate module. Feature code lives in
//   client.js    — host IPC (invokeHost + error classification)
//   dom.js       — generic DOM / form helpers
//   welcome.js   — welcome screen (create / open / recent projects)
//   workspace.js — workspace shell, document tree, block editor
//   palette.js   — command palette / AI context menu
//   review.js    — patch review (hunk accept/reject)
//   drawers.js   — side drawers (insights / timeline / compare / styles /
//                  providers / candidates / versions / backup wizard)
// This file keeps the shared mutable `state`, module-level constants, the
// bootstrap flow, global event wiring and the long-running save / AI /
// summary pipelines, then re-exports the public surface consumed by the
// browser harness and the unit tests.
import {
  SAVE_DEBOUNCE_MS,
  normalizeHostError,
  selectInitialDocument,
  blocksForDocument,
  isEditableBlock,
  applySaveResponse,
  buildSaveBlockRequest,
} from "./frontend-state.js";
import {
  PROVIDER_PRESETS,
  defaultProviderSettings,
  credentialReference,
  providerRequiresCredential,
  matchesDesktopRetryTarget,
  retryableOperationFailure,
  runDesktopOperation,
  createDesktopReview,
} from "./operation-client.js";
import { t, setLocale } from "./i18n.js";
import { announceLive } from "./a11y.js";
import { invokeHost } from "./client.js";
import { element, button } from "./dom.js";
import { setupWindowControls, loadRecentProjects, renderWelcome } from "./welcome.js";
import { renderWorkspace, operationCommandsDisabled } from "./workspace.js";
import { openCommandPalette, closeCommandSurfaces, executeRegisteredCommand, commandPaletteModal } from "./palette.js";
import { aiReviewView, announceReviewState } from "./review.js";
import {
  loadVersionHistory,
  backupRestoreWizard,
  compareBlockRow,
  compareDrawer,
  compareResultView,
  fpsMonitor,
  insightsDrawer,
  timelineDrawer,
  toggleBackupWizard,
  virtualizeDiffRows,
} from "./drawers.js";

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
  state,
  app,
  AI_OPERATION_COMMANDS,
  TIMELINE_STATE_COLORS,
  TIMELINE_STATE_LABELS,
  // shared plumbing consumed by feature modules
  element,
  invokeHost,
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
  refreshReviewCandidates,
  persistProviderSettings,
  refreshProviderSecretStatus,
  refreshTrustedModelEndpoints,
  // re-exported public surface (browser harness + unit tests)
  aiReviewView,
  announceReviewState,
  backupRestoreWizard,
  commandPaletteModal,
  compareBlockRow,
  compareDrawer,
  compareResultView,
  fpsMonitor,
  insightsDrawer,
  timelineDrawer,
  toggleBackupWizard,
  virtualizeDiffRows,
  loadVersionHistory,
};
