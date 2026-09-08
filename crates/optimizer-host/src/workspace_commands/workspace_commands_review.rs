use std::collections::{BTreeMap, BTreeSet};

use serde::Deserialize;

use optimizer_store::{
    AppendReviewEvent, ApplyBlockEdit, CommitRecord, CreateReviewCandidateBranch,
    OperationArtifactKind, OperationRunRecord, RestoreSnapshot, ReviewCandidateBranchRecord,
    ReviewCandidateSummaryRecord, ReviewDecision, ReviewEventKind, ReviewEventRecord,
    ReviewSessionRecord, ReviewSessionStatus, SnapshotRecord,
};

use super::*;

pub(crate) fn list_review_candidates(
    store: &OptimizerStore,
    project_id: &str,
) -> Result<Vec<ReviewCandidateSummary>, WorkspaceCommandError> {
    store
        .list_review_candidates(project_id, 100)?
        .into_iter()
        .map(review_candidate_summary)
        .collect()
}

pub(crate) fn load_review_candidate(
    store: &OptimizerStore,
    project_id: &str,
    proposal_id: &str,
) -> Result<ReviewCandidateDetail, WorkspaceCommandError> {
    if proposal_id.trim().is_empty() || proposal_id.len() > 200 {
        return Err(WorkspaceCommandError::Validation(
            "Proposal id is invalid".into(),
        ));
    }
    let loaded = load_stored_review_candidate(store, project_id, proposal_id)?;
    let decisions = loaded
        .decisions
        .into_iter()
        .map(|(id, decision)| {
            (
                id,
                decision
                    .map(|value| value.as_str().to_owned())
                    .unwrap_or_else(|| "pending".into()),
            )
        })
        .collect();
    Ok(ReviewCandidateDetail {
        schema_version: WORKSPACE_SCHEMA_VERSION,
        summary: review_candidate_summary(loaded.summary)?,
        proposal: loaded.payload,
        session: ReviewCandidateSession {
            proposal_id: loaded.session.proposal_id,
            proposal_hash: loaded.proposal.proposal_hash,
            revision: loaded.session.revision,
            status: loaded.session.status.as_str().into(),
            decisions,
        },
    })
}

pub(crate) fn apply_reviewed_proposal(
    store: &mut OptimizerStore,
    project_id: &str,
    main_branch_id: &str,
    spec: &ApplyReviewedProposalSpec,
) -> Result<ApplyReviewedProposalResponse, WorkspaceCommandError> {
    if spec.proposal_id.trim().is_empty()
        || spec.proposal_id.len() > 200
        || spec.expected_review_revision < 0
    {
        return Err(WorkspaceCommandError::Validation(
            "Proposal id and expected review revision are invalid".into(),
        ));
    }
    let session = store.get_review_session(&spec.proposal_id)?;
    if session.revision != spec.expected_review_revision
        || session.status != ReviewSessionStatus::Ready
    {
        return Err(WorkspaceCommandError::Store(StoreError::StateConflict {
            entity: "patch_review",
            id: spec.proposal_id.clone(),
            expected_revision: spec.expected_review_revision,
            actual_revision: session.revision,
            expected_state: ReviewSessionStatus::Ready.as_str().into(),
            actual_state: session.status.as_str().into(),
        }));
    }
    let run = store.get_operation_run(&session.run_id)?;
    if run.project_id != project_id {
        return Err(WorkspaceCommandError::Validation(
            "Proposal belongs to another project".into(),
        ));
    }
    let artifact = store.get_operation_artifact(&run.id)?;
    if artifact.id != spec.proposal_id || artifact.kind != OperationArtifactKind::PatchProposal {
        return Err(WorkspaceCommandError::Validation(
            "Proposal artifact binding is invalid".into(),
        ));
    }
    let payload: Value =
        serde_json::from_str(&artifact.payload_json).map_err(WorkspaceCommandError::Json)?;
    verify_stored_proposal_hash(&payload, &artifact.binding_hash)?;
    let proposal: StoredPatchProposal =
        serde_json::from_value(payload).map_err(WorkspaceCommandError::Json)?;
    if proposal.schema_version != 2
        || proposal.id != spec.proposal_id
        || proposal.operation_run_id != run.id
        || proposal.base_commit_id != run.base_commit_id
        || proposal.proposal_hash != artifact.binding_hash
        || proposal.status != "review"
    {
        return Err(WorkspaceCommandError::Validation(
            "Stored proposal metadata is not bound to its operation".into(),
        ));
    }

    let project = store.get_project(project_id)?;
    if project.head_commit_id != run.base_commit_id {
        return Err(WorkspaceCommandError::Store(StoreError::StateConflict {
            entity: "operation_base_commit",
            id: run.id.clone(),
            expected_revision: spec.expected_review_revision,
            actual_revision: session.revision,
            expected_state: run.base_commit_id.clone(),
            actual_state: project.head_commit_id,
        }));
    }
    let current = store.get_project_block(project_id, &proposal.target.block_id)?;
    if current.document_id != proposal.target.document_id
        || current.revision != proposal.target.base_revision
        || current.content_hash != proposal.target.base_hash
        || current.locked
        || proposal.target.from.block_id != current.id
        || proposal.target.to.block_id != current.id
        || proposal.target.from.offset > proposal.target.to.offset
    {
        return Err(WorkspaceCommandError::Store(StoreError::Conflict {
            entity: "proposal_target",
            id: current.id,
            expected_revision: proposal.target.base_revision,
            actual_revision: current.revision,
            expected_hash: proposal.target.base_hash,
            actual_hash: current.content_hash,
        }));
    }

    let events = store.list_review_events(&spec.proposal_id)?;
    validate_proposal_hunks(&current.plain_text, &proposal.target, &proposal.hunks)?;
    let decisions = complete_review_decisions(
        &proposal.hunks,
        replay_review_decisions(&proposal.hunks, &events)?,
    )?;
    validate_atomic_decisions(&proposal.hunks, &decisions)?;

    let mut accepted = Vec::new();
    let mut rejected = Vec::new();
    for hunk in &proposal.hunks {
        let decision = decisions
            .get(&hunk.id)
            .ok_or_else(|| invariant_violation("review decisions must cover every proposal hunk"))?;
        match decision {
            ReviewDecision::Accepted => accepted.push(hunk),
            ReviewDecision::Rejected => rejected.push(hunk.id.clone()),
        }
    }
    if accepted.is_empty() {
        return Err(WorkspaceCommandError::Validation(
            "A proposal with no accepted hunks must be rejected, not applied".into(),
        ));
    }
    let next_plain_text = apply_proposal_hunks(&current.plain_text, &accepted)?;
    if next_plain_text.len() > MAX_PLAIN_TEXT_BYTES {
        return Err(WorkspaceCommandError::Validation(format!(
            "Block plain text exceeds {MAX_PLAIN_TEXT_BYTES} bytes"
        )));
    }
    let mut next_content: Value =
        serde_json::from_str(&current.content_json).map_err(WorkspaceCommandError::Json)?;
    let content = next_content.as_object_mut().ok_or_else(|| {
        WorkspaceCommandError::Validation("Stored Block content must be an object".into())
    })?;
    content.remove("text");
    content.insert(
        "content".into(),
        if next_plain_text.is_empty() {
            Value::Array(Vec::new())
        } else {
            json!([{ "type": "text", "text": next_plain_text }])
        },
    );
    let content_json = serde_json::to_string(&next_content).map_err(WorkspaceCommandError::Json)?;
    let content_hash = block_content_hash(
        &current.kind,
        &next_content,
        &next_plain_text,
        current.locked,
    )?;
    let documents = store.list_documents(project_id)?;
    let blocks = store.list_blocks(project_id)?;
    let root_hash = project_root_hash(
        documents.iter(),
        blocks.iter().map(|block| {
            (
                block.id.as_str(),
                if block.id == current.id {
                    content_hash.as_str()
                } else {
                    block.content_hash.as_str()
                },
            )
        }),
    );
    let occurred_at = now()?;
    let commit_id = generated_id("commit");
    let accepted_ids = accepted
        .iter()
        .map(|hunk| hunk.id.clone())
        .collect::<Vec<_>>();
    let review_event = AppendReviewEvent {
        id: generated_id("review-event"),
        proposal_id: spec.proposal_id.clone(),
        expected_revision: spec.expected_review_revision,
        expected_status: ReviewSessionStatus::Ready,
        kind: ReviewEventKind::Apply,
        next_status: ReviewSessionStatus::Applied,
        hunk_id: None,
        decision: None,
        payload_json: Some(
            serde_json::to_string(&json!({
                "commitId": commit_id,
                "acceptedHunkIds": accepted_ids.clone(),
                "rejectedHunkIds": rejected.clone(),
            }))
            .map_err(WorkspaceCommandError::Json)?,
        ),
        occurred_at: occurred_at.clone(),
    };
    let command = ApplyBlockEdit {
        edit_id: generated_id("edit"),
        commit_id,
        branch_id: main_branch_id.into(),
        expected_head_commit_id: run.base_commit_id,
        expected_project_revision: project.revision,
        block_id: current.id,
        expected_revision: current.revision,
        expected_hash: current.content_hash,
        new_content_json: content_json,
        new_plain_text: next_plain_text,
        new_content_hash: content_hash,
        new_root_hash: root_hash,
        reason: "ai_accept".into(),
        actor_type: "model".into(),
        actor_id: Some(run.id.clone()),
        occurred_at,
        review_event: Some(review_event),
    };
    let receipt = store.apply_block_edit(&command)?;
    Ok(ApplyReviewedProposalResponse {
        schema_version: WORKSPACE_SCHEMA_VERSION,
        proposal_id: spec.proposal_id.clone(),
        run_id: run.id,
        review_revision: spec.expected_review_revision + 1,
        accepted_hunks: accepted_ids.len(),
        rejected_hunks: rejected.len(),
        save: save_response(store, project_id, receipt)?,
    })
}

pub(crate) fn create_review_candidate_branch(
    store: &mut OptimizerStore,
    project_id: &str,
    spec: &CreateReviewCandidateBranchSpec,
) -> Result<CreateReviewCandidateBranchResponse, WorkspaceCommandError> {
    let branch_name = spec.branch_name.trim();
    if spec.proposal_id.trim().is_empty()
        || spec.proposal_id.len() > 200
        || spec.expected_review_revision < 0
        || branch_name.is_empty()
        || branch_name.chars().count() > 120
    {
        return Err(WorkspaceCommandError::Validation(
            "Candidate proposal, review revision and branch name are invalid".into(),
        ));
    }
    let loaded = load_stored_review_candidate(store, project_id, &spec.proposal_id)?;
    if loaded.session.revision != spec.expected_review_revision
        || loaded.session.status != ReviewSessionStatus::Ready
    {
        return Err(WorkspaceCommandError::Store(StoreError::StateConflict {
            entity: "patch_review",
            id: spec.proposal_id.clone(),
            expected_revision: spec.expected_review_revision,
            actual_revision: loaded.session.revision,
            expected_state: ReviewSessionStatus::Ready.as_str().into(),
            actual_state: loaded.session.status.as_str().into(),
        }));
    }
    if loaded.summary.candidate_branch.is_some() {
        return Err(WorkspaceCommandError::Validation(
            "Candidate already has a branch".into(),
        ));
    }
    let project = store.get_project(project_id)?;
    if project.head_commit_id != loaded.run.base_commit_id {
        return Err(WorkspaceCommandError::Store(StoreError::StateConflict {
            entity: "candidate_branch_base",
            id: spec.proposal_id.clone(),
            expected_revision: spec.expected_review_revision,
            actual_revision: loaded.session.revision,
            expected_state: loaded.run.base_commit_id,
            actual_state: project.head_commit_id,
        }));
    }
    let current = store.get_project_block(project_id, &loaded.proposal.target.block_id)?;
    if current.document_id != loaded.proposal.target.document_id
        || current.revision != loaded.proposal.target.base_revision
        || current.content_hash != loaded.proposal.target.base_hash
        || current.locked
    {
        return Err(WorkspaceCommandError::Store(StoreError::Conflict {
            entity: "candidate_branch_target",
            id: current.id,
            expected_revision: loaded.proposal.target.base_revision,
            actual_revision: current.revision,
            expected_hash: loaded.proposal.target.base_hash,
            actual_hash: current.content_hash,
        }));
    }
    validate_proposal_hunks(
        &current.plain_text,
        &loaded.proposal.target,
        &loaded.proposal.hunks,
    )?;
    let decisions = complete_review_decisions(&loaded.proposal.hunks, loaded.decisions)?;
    validate_atomic_decisions(&loaded.proposal.hunks, &decisions)?;
    let accepted = loaded
        .proposal
        .hunks
        .iter()
        .filter(|hunk| decisions.get(&hunk.id) == Some(&ReviewDecision::Accepted))
        .collect::<Vec<_>>();
    if accepted.is_empty() {
        return Err(WorkspaceCommandError::Validation(
            "A candidate branch requires at least one accepted hunk".into(),
        ));
    }
    let next_plain_text = apply_proposal_hunks(&current.plain_text, &accepted)?;
    if next_plain_text.len() > MAX_PLAIN_TEXT_BYTES {
        return Err(WorkspaceCommandError::Validation(format!(
            "Block plain text exceeds {MAX_PLAIN_TEXT_BYTES} bytes"
        )));
    }
    let mut next_content: Value =
        serde_json::from_str(&current.content_json).map_err(WorkspaceCommandError::Json)?;
    let content = next_content.as_object_mut().ok_or_else(|| {
        WorkspaceCommandError::Validation("Stored Block content must be an object".into())
    })?;
    content.remove("text");
    content.insert(
        "content".into(),
        if next_plain_text.is_empty() {
            Value::Array(Vec::new())
        } else {
            json!([{ "type": "text", "text": next_plain_text }])
        },
    );
    let content_json = serde_json::to_string(&next_content).map_err(WorkspaceCommandError::Json)?;
    let content_hash = block_content_hash(
        &current.kind,
        &next_content,
        &next_plain_text,
        current.locked,
    )?;
    let documents = store.list_documents(project_id)?;
    let blocks = store.list_blocks(project_id)?;
    let root_hash = project_root_hash(
        documents.iter(),
        blocks.iter().map(|block| {
            (
                block.id.as_str(),
                if block.id == current.id {
                    content_hash.as_str()
                } else {
                    block.content_hash.as_str()
                },
            )
        }),
    );
    let occurred_at = now()?;
    let branch = store.create_review_candidate_branch(&CreateReviewCandidateBranch {
        proposal_id: spec.proposal_id.clone(),
        expected_review_revision: spec.expected_review_revision,
        project_id: project_id.into(),
        expected_project_head_commit_id: project.head_commit_id.clone(),
        branch_id: generated_id("branch"),
        branch_name: branch_name.into(),
        commit_id: generated_id("commit"),
        snapshot_id: generated_id("snapshot"),
        target_document_id: current.document_id,
        target_block_id: current.id,
        expected_block_revision: current.revision,
        expected_block_hash: current.content_hash,
        new_content_json: content_json,
        new_plain_text: next_plain_text,
        new_content_hash: content_hash,
        new_root_hash: root_hash,
        occurred_at,
    })?;
    Ok(CreateReviewCandidateBranchResponse {
        schema_version: WORKSPACE_SCHEMA_VERSION,
        branch: review_candidate_branch(branch),
        main_head_commit_id: project.head_commit_id,
    })
}

pub(crate) fn create_checkpoint(
    store: &mut OptimizerStore,
    project_id: &str,
) -> Result<CheckpointSummary, WorkspaceCommandError> {
    let created_at = now()?;
    let record = store.create_head_snapshot(&generated_id("snapshot"), project_id, &created_at)?;
    Ok(checkpoint_summary(record))
}

pub(crate) fn load_version_history(
    store: &OptimizerStore,
    project_id: &str,
) -> Result<VersionHistory, WorkspaceCommandError> {
    let project = store.get_project(project_id)?;
    let commits = store
        .list_commits(project_id)?
        .into_iter()
        .map(version_commit)
        .collect();
    let checkpoints = store
        .list_snapshots(project_id)?
        .into_iter()
        .map(checkpoint_summary)
        .collect();
    Ok(VersionHistory {
        schema_version: WORKSPACE_SCHEMA_VERSION,
        project_id: project.id,
        head_commit_id: project.head_commit_id,
        commits,
        checkpoints,
    })
}

/// v0.6.0 timeline: derive a linear, `created_at`-descending event stream
/// from the existing version history checkpoints, optionally associating
/// each checkpoint with the operation run that was initiated from its
/// commit. Reuses `load_version_history` (D4) so the data source stays
/// identical to the version drawer; operation runs are matched via
/// `operation_run.base_commit_id == checkpoint.commit_id`.
///
/// v0.7.0 Stage 1 (D1): each event now also carries its
/// [`parent_commit_ids`](TimelineEvent::parent_commit_ids) — the ordered
/// list of parent commit ids drawn from the existing `commit_parent` table
/// (no schema change). Multi-parent entries surface branch merge points so
/// the frontend SVG overlay can render connection lines without introducing
/// CRDT state. Empty for the seed commit and single-element for linear
/// commits, preserving the v0.6.0 backward-compatible baseline.
pub fn list_timeline_events(
    store: &OptimizerStore,
    project_id: &str,
) -> Result<TimelineEventsResponse, WorkspaceCommandError> {
    let history = load_version_history(store, project_id)?;
    // list_operation_runs orders by started_at DESC, so the first run seen
    // for a given base_commit_id is the latest one. Cap at the store's max
    // page size (200) — timelines cover recent activity, not full archives.
    let runs = store
        .list_operation_runs(project_id, 200, 0)
        .map_err(WorkspaceCommandError::Store)?;
    let mut latest_run_by_base: BTreeMap<&str, &OperationRunRecord> = BTreeMap::new();
    for run in &runs {
        latest_run_by_base
            .entry(run.base_commit_id.as_str())
            .or_insert(run);
    }
    // v0.7.0 Stage 1 (D1): preload each checkpoint commit's parent ids via a
    // single batched `IN (...)` query (chunks of at most 200 ids) instead of
    // one query per commit (N+1). Empty for the seed commit and single-element
    // for linear commits — both cases stay backward compatible with the
    // v0.6.0 linear timeline renderer.
    let mut distinct_commit_ids: Vec<String> = history
        .checkpoints
        .iter()
        .map(|checkpoint| checkpoint.commit_id.clone())
        .collect::<BTreeSet<_>>()
        .into_iter()
        .collect();
    distinct_commit_ids.sort();
    let parents_by_commit = store
        .list_commit_parents_batch(project_id, &distinct_commit_ids)
        .map_err(WorkspaceCommandError::Store)?;
    let mut events: Vec<TimelineEvent> = history
        .checkpoints
        .into_iter()
        .map(|checkpoint| {
            let operation_summary = latest_run_by_base
                .get(checkpoint.commit_id.as_str())
                .map(|run| OperationSummary {
                    operation_id: run.id.clone(),
                    state: run.state.as_str().to_owned(),
                    operation_type: None,
                });
            let parent_commit_ids = parents_by_commit
                .get(&checkpoint.commit_id)
                .cloned()
                .unwrap_or_default();
            TimelineEvent {
                checkpoint_id: checkpoint.id,
                commit_id: checkpoint.commit_id,
                created_at: checkpoint.created_at,
                operation_summary,
                parent_commit_ids,
            }
        })
        .collect();
    events.sort_by(|a, b| b.created_at.cmp(&a.created_at));
    let total_count = events.len();
    Ok(TimelineEventsResponse {
        schema_version: WORKSPACE_SCHEMA_VERSION,
        events,
        total_count,
    })
}

pub(crate) fn restore_checkpoint(
    store: &mut OptimizerStore,
    project_id: &str,
    main_branch_id: &str,
    spec: &RestoreCheckpointSpec,
) -> Result<RestoreCheckpointResponse, WorkspaceCommandError> {
    if spec.checkpoint_id.trim().is_empty() || spec.checkpoint_id.len() > 200 {
        return Err(WorkspaceCommandError::Validation(
            "Checkpoint id is invalid".into(),
        ));
    }
    let receipt = store.restore_snapshot(&RestoreSnapshot {
        snapshot_id: spec.checkpoint_id.clone(),
        branch_id: main_branch_id.into(),
        new_commit_id: generated_id("commit"),
        edit_id_prefix: generated_id("restore-edit"),
        actor_type: "user".into(),
        actor_id: None,
        occurred_at: now()?,
    })?;
    let workspace = load_project_workspace(store, project_id, main_branch_id)?;
    Ok(RestoreCheckpointResponse {
        schema_version: WORKSPACE_SCHEMA_VERSION,
        checkpoint_id: receipt.snapshot_id,
        commit_id: receipt.commit_id,
        previous_head_commit_id: receipt.previous_head_commit_id,
        restored_root_hash: receipt.restored_root_hash,
        changed_blocks: receipt.changed_blocks,
        workspace,
    })
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
pub struct CompareDocumentsSpec {
    pub snapshot_id_a: String,
    pub snapshot_id_b: String,
    pub document_id_a: String,
    pub document_id_b: String,
}

pub(crate) fn compare_documents(
    store: &OptimizerStore,
    project_id: &str,
    spec: &CompareDocumentsSpec,
) -> Result<optimizer_store::DocumentDiffResult, WorkspaceCommandError> {
    for (field, value) in [
        ("snapshotIdA", spec.snapshot_id_a.as_str()),
        ("snapshotIdB", spec.snapshot_id_b.as_str()),
        ("documentIdA", spec.document_id_a.as_str()),
        ("documentIdB", spec.document_id_b.as_str()),
    ] {
        if value.trim().is_empty() || value.len() > 200 {
            return Err(WorkspaceCommandError::Validation(format!(
                "{field} must be a non-empty string of at most 200 characters"
            )));
        }
    }

    let record_a = store.get_snapshot(&spec.snapshot_id_a)?;
    if record_a.project_id != project_id {
        return Err(WorkspaceCommandError::Validation(
            "snapshotIdA does not belong to the current project".into(),
        ));
    }
    let record_b = store.get_snapshot(&spec.snapshot_id_b)?;
    if record_b.project_id != project_id {
        return Err(WorkspaceCommandError::Validation(
            "snapshotIdB does not belong to the current project".into(),
        ));
    }

    let snapshot_a = store.decode_snapshot_record(&record_a)?;
    let snapshot_b = store.decode_snapshot_record(&record_b)?;

    Ok(optimizer_store::compare_documents(
        &snapshot_a,
        &snapshot_b,
        &spec.document_id_a,
        &spec.document_id_b,
    ))
}

#[derive(Debug, Deserialize)]
#[serde(rename_all = "camelCase")]
struct StoredPatchProposal {
    schema_version: u32,
    id: String,
    operation_run_id: String,
    base_commit_id: String,
    target: StoredProposalTarget,
    hunks: Vec<StoredProposalHunk>,
    status: String,
    proposal_hash: String,
}

#[derive(Debug, Deserialize)]
#[serde(rename_all = "camelCase")]
struct StoredProposalTarget {
    document_id: String,
    block_id: String,
    base_revision: i64,
    base_hash: String,
    from: StoredTextAnchor,
    to: StoredTextAnchor,
}

#[derive(Debug, Deserialize)]
#[serde(rename_all = "camelCase")]
pub(crate) struct StoredProposalHunk {
    pub(crate) id: String,
    pub(crate) from: StoredTextAnchor,
    pub(crate) to: StoredTextAnchor,
    pub(crate) original: String,
    pub(crate) replacement: String,
    pub(crate) atomic_group: Option<String>,
}

#[derive(Debug, Deserialize)]
#[serde(rename_all = "camelCase")]
pub(crate) struct StoredTextAnchor {
    pub(crate) block_id: String,
    pub(crate) offset: usize,
}

struct LoadedStoredReviewCandidate {
    summary: ReviewCandidateSummaryRecord,
    session: ReviewSessionRecord,
    run: OperationRunRecord,
    payload: Value,
    proposal: StoredPatchProposal,
    decisions: BTreeMap<String, Option<ReviewDecision>>,
}

fn load_stored_review_candidate(
    store: &OptimizerStore,
    project_id: &str,
    proposal_id: &str,
) -> Result<LoadedStoredReviewCandidate, WorkspaceCommandError> {
    let summary = store.get_review_candidate_summary(proposal_id)?;
    if summary.project_id != project_id {
        return Err(WorkspaceCommandError::Validation(
            "Proposal belongs to another project".into(),
        ));
    }
    let session = store.get_review_session(proposal_id)?;
    let run = store.get_operation_run(&session.run_id)?;
    let artifact = store.get_operation_artifact_by_id(proposal_id)?;
    if session.run_id != summary.run_id
        || run.id != summary.run_id
        || run.project_id != project_id
        || artifact.run_id != run.id
        || artifact.kind != OperationArtifactKind::PatchProposal
    {
        return Err(WorkspaceCommandError::Store(
            StoreError::InvariantViolation(
                "review candidate store bindings are inconsistent".into(),
            ),
        ));
    }
    let payload: Value =
        serde_json::from_str(&artifact.payload_json).map_err(WorkspaceCommandError::Json)?;
    verify_stored_proposal_hash(&payload, &artifact.binding_hash)?;
    let proposal: StoredPatchProposal =
        serde_json::from_value(payload.clone()).map_err(WorkspaceCommandError::Json)?;
    if proposal.schema_version != 2
        || proposal.id != proposal_id
        || proposal.operation_run_id != run.id
        || proposal.base_commit_id != run.base_commit_id
        || proposal.proposal_hash != artifact.binding_hash
        || proposal.status != "review"
        || proposal.target.document_id != summary.target_document_id
        || proposal.target.block_id != summary.target_block_id
        || proposal.hunks.len() != usize::try_from(summary.hunk_count).unwrap_or(usize::MAX)
    {
        return Err(WorkspaceCommandError::Validation(
            "Stored proposal metadata is not bound to its candidate summary".into(),
        ));
    }
    let events = store.list_review_events(proposal_id)?;
    let decisions = replay_review_decisions(&proposal.hunks, &events)?;
    let all_decided = decisions.values().all(Option::is_some);
    if matches!(
        session.status,
        ReviewSessionStatus::Ready | ReviewSessionStatus::Applied
    ) && !all_decided
        || session.status == ReviewSessionStatus::Review && all_decided
    {
        return Err(WorkspaceCommandError::Store(
            StoreError::InvariantViolation(
                "review candidate status disagrees with replayed decisions".into(),
            ),
        ));
    }
    Ok(LoadedStoredReviewCandidate {
        summary,
        session,
        run,
        payload,
        proposal,
        decisions,
    })
}

pub(crate) fn replay_review_decisions(
    hunks: &[StoredProposalHunk],
    events: &[ReviewEventRecord],
) -> Result<BTreeMap<String, Option<ReviewDecision>>, WorkspaceCommandError> {
    let mut decisions = hunks
        .iter()
        .map(|hunk| (hunk.id.clone(), None))
        .collect::<BTreeMap<_, _>>();
    for event in events {
        if event.kind != ReviewEventKind::Decision {
            continue;
        }
        let hunk_id = event.hunk_id.as_deref().ok_or_else(|| {
            WorkspaceCommandError::Validation("Decision event has no hunk id".into())
        })?;
        let decision = event.decision.ok_or_else(|| {
            WorkspaceCommandError::Validation("Decision event has no decision".into())
        })?;
        let selected = hunks
            .iter()
            .find(|hunk| hunk.id == hunk_id)
            .ok_or_else(|| {
                WorkspaceCommandError::Validation(
                    "Decision event references an unknown hunk".into(),
                )
            })?;
        if let Some(group) = selected.atomic_group.as_deref() {
            if group.trim().is_empty() {
                return Err(WorkspaceCommandError::Validation(
                    "Atomic proposal group cannot be empty".into(),
                ));
            }
            for hunk in hunks
                .iter()
                .filter(|hunk| hunk.atomic_group.as_deref() == Some(group))
            {
                decisions.insert(hunk.id.clone(), Some(decision));
            }
        } else {
            decisions.insert(selected.id.clone(), Some(decision));
        }
    }
    Ok(decisions)
}

pub(crate) fn complete_review_decisions(
    hunks: &[StoredProposalHunk],
    decisions: BTreeMap<String, Option<ReviewDecision>>,
) -> Result<BTreeMap<String, ReviewDecision>, WorkspaceCommandError> {
    if decisions.len() != hunks.len()
        || hunks
            .iter()
            .any(|hunk| !matches!(decisions.get(&hunk.id), Some(Some(_))))
    {
        return Err(WorkspaceCommandError::Validation(
            "Review decisions do not cover exactly every proposal hunk".into(),
        ));
    }
    decisions
        .into_iter()
        .map(|(id, decision)| {
            decision.map(|value| (id, value)).ok_or_else(|| {
                WorkspaceCommandError::Validation("Review decision is still pending".into())
            })
        })
        .collect()
}

fn review_candidate_summary(
    record: ReviewCandidateSummaryRecord,
) -> Result<ReviewCandidateSummary, WorkspaceCommandError> {
    Ok(ReviewCandidateSummary {
        schema_version: WORKSPACE_SCHEMA_VERSION,
        proposal_id: record.proposal_id,
        run_id: record.run_id,
        operation_intent_id: record.operation_intent_id,
        provider_id: record.provider_id,
        model: record.model,
        base_commit_id: record.base_commit_id,
        target_document_id: record.target_document_id,
        target_block_id: record.target_block_id,
        hunk_count: usize::try_from(record.hunk_count).map_err(|_| {
            WorkspaceCommandError::Store(StoreError::InvariantViolation(
                "review candidate hunk count is invalid".into(),
            ))
        })?,
        summary: record.summary,
        revision: record.revision,
        status: record.status.as_str().into(),
        created_at: record.created_at,
        updated_at: record.updated_at,
        candidate_branch: record.candidate_branch.map(review_candidate_branch),
    })
}

fn review_candidate_branch(record: ReviewCandidateBranchRecord) -> ReviewCandidateBranch {
    ReviewCandidateBranch {
        proposal_id: record.proposal_id,
        branch_id: record.branch_id,
        branch_name: record.branch_name,
        commit_id: record.commit_id,
        snapshot_id: record.snapshot_id,
        created_at: record.created_at,
    }
}

fn verify_stored_proposal_hash(
    payload: &Value,
    binding_hash: &str,
) -> Result<(), WorkspaceCommandError> {
    let mut canonical = payload.clone();
    let object = canonical.as_object_mut().ok_or_else(|| {
        WorkspaceCommandError::Validation("Stored proposal payload must be an object".into())
    })?;
    let declared = object
        .remove("proposalHash")
        .and_then(|value| value.as_str().map(str::to_owned))
        .ok_or_else(|| {
            WorkspaceCommandError::Validation("Stored proposal has no proposalHash".into())
        })?;
    let encoded = serde_json::to_vec(&canonical).map_err(WorkspaceCommandError::Json)?;
    let calculated = sha256(&encoded);
    if declared != binding_hash || calculated != binding_hash {
        return Err(WorkspaceCommandError::Validation(
            "Stored proposal hash verification failed".into(),
        ));
    }
    Ok(())
}

fn validate_proposal_hunks(
    source: &str,
    target: &StoredProposalTarget,
    hunks: &[StoredProposalHunk],
) -> Result<(), WorkspaceCommandError> {
    if hunks.is_empty() || hunks.len() > 500 {
        return Err(WorkspaceCommandError::Validation(
            "Stored proposal must contain 1..=500 hunks".into(),
        ));
    }
    utf16_to_byte(source, target.from.offset).ok_or_else(|| {
        WorkspaceCommandError::Validation("Proposal target start is not a UTF-16 boundary".into())
    })?;
    utf16_to_byte(source, target.to.offset).ok_or_else(|| {
        WorkspaceCommandError::Validation("Proposal target end is not a UTF-16 boundary".into())
    })?;
    let mut ids = BTreeSet::new();
    let mut previous_end = target.from.offset;
    let mut previous_insertion = None;
    for hunk in hunks {
        if hunk.id.trim().is_empty() || !ids.insert(hunk.id.as_str()) {
            return Err(WorkspaceCommandError::Validation(
                "Proposal hunk ids must be non-empty and unique".into(),
            ));
        }
        if hunk.from.block_id != target.block_id
            || hunk.to.block_id != target.block_id
            || hunk.from.offset > hunk.to.offset
            || hunk.from.offset < target.from.offset
            || hunk.to.offset > target.to.offset
            || hunk.from.offset < previous_end
            || (hunk.from.offset == hunk.to.offset && previous_insertion == Some(hunk.from.offset))
        {
            return Err(WorkspaceCommandError::Validation(
                "Proposal hunks are outside the target, ambiguous or overlapping".into(),
            ));
        }
        let from = utf16_to_byte(source, hunk.from.offset).ok_or_else(|| {
            WorkspaceCommandError::Validation("Hunk start is not a UTF-16 boundary".into())
        })?;
        let to = utf16_to_byte(source, hunk.to.offset).ok_or_else(|| {
            WorkspaceCommandError::Validation("Hunk end is not a UTF-16 boundary".into())
        })?;
        if source.get(from..to) != Some(hunk.original.as_str()) {
            let actual = source.get(from..to).unwrap_or_default();
            return Err(WorkspaceCommandError::Store(StoreError::Conflict {
                entity: "proposal_hunk",
                id: hunk.id.clone(),
                expected_revision: target.base_revision,
                actual_revision: target.base_revision,
                expected_hash: sha256(hunk.original.as_bytes()),
                actual_hash: sha256(actual.as_bytes()),
            }));
        }
        previous_end = hunk.to.offset;
        previous_insertion = (hunk.from.offset == hunk.to.offset).then_some(hunk.from.offset);
    }
    Ok(())
}

fn validate_atomic_decisions(
    hunks: &[StoredProposalHunk],
    decisions: &BTreeMap<String, ReviewDecision>,
) -> Result<(), WorkspaceCommandError> {
    let mut groups = BTreeMap::new();
    for hunk in hunks {
        let Some(group) = hunk.atomic_group.as_deref() else {
            continue;
        };
        if group.trim().is_empty() {
            return Err(WorkspaceCommandError::Validation(
                "Atomic proposal group cannot be empty".into(),
            ));
        }
        let decision = decisions[&hunk.id];
        if groups
            .insert(group, decision)
            .is_some_and(|prior| prior != decision)
        {
            return Err(WorkspaceCommandError::Validation(
                "Atomic proposal group has inconsistent decisions".into(),
            ));
        }
    }
    Ok(())
}

fn apply_proposal_hunks(
    source: &str,
    accepted: &[&StoredProposalHunk],
) -> Result<String, WorkspaceCommandError> {
    let mut output = source.to_owned();
    let mut descending = accepted.to_vec();
    descending.sort_unstable_by(|left, right| {
        right
            .from
            .offset
            .cmp(&left.from.offset)
            .then_with(|| right.to.offset.cmp(&left.to.offset))
    });
    for hunk in descending {
        let from = utf16_to_byte(&output, hunk.from.offset).ok_or_else(|| {
            WorkspaceCommandError::Validation("Hunk start is not a UTF-16 boundary".into())
        })?;
        let to = utf16_to_byte(&output, hunk.to.offset).ok_or_else(|| {
            WorkspaceCommandError::Validation("Hunk end is not a UTF-16 boundary".into())
        })?;
        output.replace_range(from..to, &hunk.replacement);
    }
    Ok(output)
}

fn utf16_to_byte(value: &str, target: usize) -> Option<usize> {
    if target == 0 {
        return Some(0);
    }
    let mut utf16 = 0;
    for (byte, character) in value.char_indices() {
        if utf16 == target {
            return Some(byte);
        }
        utf16 += character.len_utf16();
        if utf16 > target {
            return None;
        }
    }
    (utf16 == target).then_some(value.len())
}

fn checkpoint_summary(record: SnapshotRecord) -> CheckpointSummary {
    CheckpointSummary {
        id: record.id,
        commit_id: record.commit_id,
        root_hash: record.root_hash,
        codec: record.codec,
        codec_version: record.codec_version,
        checksum: record.checksum,
        created_at: record.created_at,
    }
}

fn version_commit(record: CommitRecord) -> VersionCommit {
    VersionCommit {
        id: record.id,
        root_hash: record.root_hash,
        reason: record.reason,
        actor_type: record.actor_type,
        actor_id: record.actor_id,
        created_at: record.created_at,
        parents: record.parents,
    }
}
