import { t } from "./i18n.js";
import {
  element,
  button,
  iconOnlyButton,
  checkboxField,
  passwordField,
  labeledInput,
  setSelectValue,
  formatDate,
} from "./dom.js";
import { invokeHost } from "./client.js";
import { normalizeHostError } from "./frontend-state.js";
import { PROVIDER_PRESETS, providerRequiresCredential, credentialReference, hydrateDesktopReviewCandidate } from "./operation-client.js";
import { estimateFees } from "./insights-pricing.js";
import { trapFocus } from "./a11y.js";
import { renderWorkspace } from "./workspace.js";
import {
  state,
  setNotice,
  flushAll,
  scheduleSummaryRefresh,
  refreshReviewCandidates,
  persistProviderSettings,
  refreshProviderSecretStatus,
  refreshTrustedModelEndpoints,
  TIMELINE_STATE_COLORS,
  TIMELINE_STATE_LABELS,
} from "./main.js";

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

export {
  attachDrawerKeyboard,
  toggleCandidates,
  candidateDrawer,
  reviewCandidateCard,
  reviewCandidateStatusLabel,
  openReviewCandidate,
  toggleVersions,
  toggleProviders,
  toggleStyles,
  toggleInsights,
  toggleRevisionMetrics,
  toggleCompare,
  toggleTimeline,
  loadTimelineEvents,
  loadInsights,
  exportDiagnostics,
  insightsDrawer,
  insightsStateLabel,
  timelineDrawer,
  assignTimelineLanes,
  buildTimelineSvgOverlay,
  timelineNodeRow,
  getRAF,
  getCAF,
  getResizeObserverCtor,
  virtualizeDiffRows,
  fpsMonitor,
  compareDrawer,
  runCompareDocuments,
  compareResultView,
  compareBlockRow,
  styleDrawer,
  knowledgeForm,
  knowledgeItemCard,
  createKnowledgeItem,
  updateKnowledgeItemStatus,
  styleSampleCard,
  pinCurrentStyleSample,
  updateStyleSampleStatus,
  providerDrawer,
  compatibleProviderPanel,
  compatibleEndpointSettingsForm,
  compatibleEndpointRegistrationForm,
  qwenProviderFields,
  ollamaProviderFields,
  probeOllamaModels,
  registerCompatibleEndpoint,
  saveCompatibleProviderSettings,
  probeCompatibleModels,
  removeCompatibleEndpoint,
  saveProviderSettings,
  deleteProviderCredential,
  loadVersionHistory,
  versionDrawer,
  restoreCheckpoint,
  toggleBackupWizard,
  resetBackupWizardForm,
  browseBackupOutputPath,
  browseBackupArchivePath,
  browseRestoreTargetDirectory,
  backupWizardStep1,
  backupWizardStep2Backup,
  backupWizardStep2Restore,
  backupWizardPreviewRows,
  backupWizardStep3Preview,
  backupWizardResultView,
  backupRestoreWizard,
  executeBackup,
  executeRestore,
};
