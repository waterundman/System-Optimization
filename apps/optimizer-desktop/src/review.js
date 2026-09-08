import { t } from "./i18n.js";
import { element, button } from "./dom.js";
import { invokeHost } from "./client.js";
import { normalizeHostError, applySaveResponse } from "./frontend-state.js";
import {
  decideDesktopHunk,
  persistDesktopReviewDecision,
  planDesktopReviewDecisions,
  rejectDesktopReview,
  compileDesktopReview,
} from "./operation-client.js";
import { announceLive } from "./a11y.js";
import { renderWorkspace } from "./workspace.js";
import { state, setNotice, refreshReviewCandidates, scheduleSummaryRefresh } from "./main.js";

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

export {
  aiReviewView,
  hunkReviewView,
  announceReviewState,
  decideCurrentHunk,
  decideAllCurrentHunks,
  deferCurrentReview,
  createCurrentCandidateBranch,
  rejectCurrentReview,
  applyCurrentReview,
};
