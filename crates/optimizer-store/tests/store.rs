use std::fs;
use std::path::{Path, PathBuf};
use std::time::{SystemTime, UNIX_EPOCH};

use optimizer_store::{
    AppendReviewEvent, ApplyBlockEdit, CURRENT_SCHEMA_VERSION, CreateSnapshot,
    MINIMUM_SQLITE_VERSION, ModelUsageRecord, NewContextPacket, NewOperationArtifact,
    NewOperationLifecycleEvent, NewOperationRun, OperationArtifactKind, OperationFailureRecord,
    OperationState, OptimizerStore, PersistOperationBundle, ProjectSeed, RestoreSnapshot,
    ReviewDecision, ReviewEventKind, ReviewSessionStatus, SeedBlock, SeedDocument, StoreError,
    encode_snapshot,
};

struct TempDatabase {
    directory: PathBuf,
    database: PathBuf,
    backup: PathBuf,
}

impl TempDatabase {
    fn new() -> Self {
        let nonce = SystemTime::now()
            .duration_since(UNIX_EPOCH)
            .unwrap()
            .as_nanos();
        let directory =
            std::env::temp_dir().join(format!("optimizer-store-{}-{nonce}", std::process::id()));
        fs::create_dir_all(&directory).unwrap();
        Self {
            database: directory.join("project.sqlite3"),
            backup: directory.join("backup.sqlite3"),
            directory,
        }
    }
}

impl Drop for TempDatabase {
    fn drop(&mut self) {
        let _ = fs::remove_dir_all(&self.directory);
    }
}

fn seed() -> ProjectSeed {
    ProjectSeed {
        project_id: "project-1".into(),
        title: "Optimizer Store Test".into(),
        language: "zh-CN".into(),
        initial_commit_id: "commit-initial".into(),
        initial_root_hash: "sha256:root-initial".into(),
        main_branch_id: "branch-main".into(),
        documents: vec![SeedDocument {
            id: "document-1".into(),
            parent_id: None,
            kind: "chapter".into(),
            title: "第一章".into(),
            order_key: "a0".into(),
        }],
        blocks: vec![SeedBlock {
            id: "block-1".into(),
            document_id: "document-1".into(),
            kind: "paragraph".into(),
            order_key: "a0".into(),
            content_json: r#"{"type":"paragraph","text":"station platform"}"#.into(),
            plain_text: "station platform".into(),
            content_hash: "sha256:block-initial".into(),
            locked: false,
        }],
        created_at: "2026-07-14T00:00:00.000Z".into(),
    }
}

fn edit(commit_id: &str, expected_revision: i64, expected_hash: &str) -> ApplyBlockEdit {
    ApplyBlockEdit {
        edit_id: format!("edit-{commit_id}"),
        commit_id: commit_id.into(),
        branch_id: "branch-main".into(),
        block_id: "block-1".into(),
        expected_revision,
        expected_hash: expected_hash.into(),
        new_content_json: r#"{"type":"paragraph","text":"harbor signal"}"#.into(),
        new_plain_text: "harbor signal".into(),
        new_content_hash: format!("sha256:block-{commit_id}"),
        new_root_hash: format!("sha256:root-{commit_id}"),
        reason: "autosave".into(),
        actor_type: "human".into(),
        actor_id: Some("user-local".into()),
        occurred_at: "2026-07-14T00:01:00.000Z".into(),
    }
}

fn operation_bundle(run_id: &str, packet_id: &str, proposal_id: &str) -> PersistOperationBundle {
    let transition = |from_state, to_state| NewOperationLifecycleEvent {
        from_state,
        to_state,
        occurred_at: "2026-07-15T00:01:00.000Z".into(),
        reason: None,
    };
    PersistOperationBundle {
        run: NewOperationRun {
            id: run_id.into(),
            operation_intent_id: format!("intent-{run_id}"),
            project_id: "project-1".into(),
            base_commit_id: "commit-initial".into(),
            provider_id: "deepseek".into(),
            model: "deepseek-v4-flash".into(),
            state: OperationState::Review,
            response_id: Some(format!("response-{run_id}")),
            finish_reason: Some("stop".into()),
            usage: Some(ModelUsageRecord {
                input_tokens: 80,
                output_tokens: 20,
                total_tokens: 100,
                cached_input_tokens: Some(12),
                reasoning_tokens: Some(5),
            }),
            failure: None,
            started_at: "2026-07-15T00:00:00.000Z".into(),
            updated_at: "2026-07-15T00:01:00.000Z".into(),
        },
        context_packet: Some(NewContextPacket {
            id: packet_id.into(),
            operation_intent_id: format!("intent-{run_id}"),
            project_id: "project-1".into(),
            base_commit_id: "commit-initial".into(),
            packet_hash: format!("sha256:{packet_id}"),
            payload_json: format!(r#"{{"id":"{packet_id}","items":[]}}"#),
            created_at: "2026-07-15T00:00:10.000Z".into(),
        }),
        lifecycle_events: vec![
            transition(OperationState::Draft, OperationState::Compiling),
            transition(OperationState::Compiling, OperationState::Preflight),
            transition(OperationState::Preflight, OperationState::Queued),
            transition(OperationState::Queued, OperationState::Streaming),
            transition(OperationState::Streaming, OperationState::Validating),
            transition(OperationState::Validating, OperationState::Review),
        ],
        artifact: Some(NewOperationArtifact {
            id: proposal_id.into(),
            kind: OperationArtifactKind::PatchProposal,
            binding_hash: format!("sha256:{proposal_id}"),
            payload_json: format!(r#"{{"id":"{proposal_id}","schemaVersion":2}}"#),
            created_at: "2026-07-15T00:01:00.000Z".into(),
        }),
    }
}

fn open_seeded(path: &Path) -> OptimizerStore {
    let mut store = OptimizerStore::open(path).unwrap();
    store.initialize_project(&seed()).unwrap();
    store
}

#[test]
fn opens_with_safe_bundled_sqlite_and_migrates_once() {
    let temp = TempDatabase::new();
    let store = OptimizerStore::open(&temp.database).unwrap();
    let diagnostics = store.diagnostics().unwrap();
    assert_eq!(diagnostics.schema_version, CURRENT_SCHEMA_VERSION);
    assert_eq!(diagnostics.sqlite_version, MINIMUM_SQLITE_VERSION);
    assert_eq!(diagnostics.journal_mode.to_ascii_lowercase(), "wal");
    assert!(diagnostics.foreign_keys);
    assert_eq!(diagnostics.synchronous, 2);
}

#[test]
fn applies_block_edit_journal_commit_and_fts_in_one_transaction() {
    let temp = TempDatabase::new();
    let mut store = open_seeded(&temp.database);
    assert_eq!(
        store
            .search_blocks("project-1", "station", 10)
            .unwrap()
            .len(),
        1
    );

    let receipt = store
        .apply_block_edit(&edit("commit-1", 0, "sha256:block-initial"))
        .unwrap();
    assert_eq!(receipt.previous_head_commit_id, "commit-initial");
    assert_eq!(receipt.new_revision, 1);

    let block = store.get_block("block-1").unwrap();
    assert_eq!(block.revision, 1);
    assert_eq!(block.plain_text, "harbor signal");
    assert!(
        store
            .search_blocks("project-1", "station", 10)
            .unwrap()
            .is_empty()
    );
    assert_eq!(
        store
            .search_blocks("project-1", "harbor", 10)
            .unwrap()
            .len(),
        1
    );

    let commits = store.list_commits("project-1").unwrap();
    assert_eq!(commits.len(), 2);
    assert_eq!(commits[1].id, "commit-1");
    assert_eq!(commits[1].parents, vec!["commit-initial"]);
    store.verify_invariants().unwrap();
}

#[test]
fn optimistic_conflict_has_no_partial_side_effects() {
    let temp = TempDatabase::new();
    let mut store = open_seeded(&temp.database);
    let error = store
        .apply_block_edit(&edit("commit-conflict", 9, "sha256:wrong"))
        .unwrap_err();
    assert!(matches!(error, StoreError::Conflict { .. }));
    assert_eq!(store.get_block("block-1").unwrap().revision, 0);
    assert_eq!(store.list_commits("project-1").unwrap().len(), 1);
    assert_eq!(
        store
            .search_blocks("project-1", "station", 10)
            .unwrap()
            .len(),
        1
    );
    store.verify_invariants().unwrap();
}

#[test]
fn stores_immutable_snapshot_bound_to_commit_root_hash() {
    let temp = TempDatabase::new();
    let mut store = open_seeded(&temp.database);
    store
        .create_snapshot(&CreateSnapshot {
            id: "snapshot-1".into(),
            project_id: "project-1".into(),
            commit_id: "commit-initial".into(),
            root_hash: "sha256:root-initial".into(),
            codec: "json".into(),
            codec_version: 1,
            payload: br#"{"blocks":1}"#.to_vec(),
            checksum: "sha256:snapshot".into(),
            created_at: "2026-07-14T00:02:00.000Z".into(),
        })
        .unwrap();
    let snapshot = store.latest_snapshot("project-1").unwrap().unwrap();
    assert_eq!(snapshot.id, "snapshot-1");
    assert_eq!(snapshot.commit_id, "commit-initial");
    drop(store);

    let connection = rusqlite::Connection::open(&temp.database).unwrap();
    let result = connection.execute(
        "UPDATE materialized_snapshot SET checksum = 'tampered' WHERE id = 'snapshot-1'",
        [],
    );
    assert!(result.is_err());
}

#[test]
fn creates_a_consistent_online_backup() {
    let temp = TempDatabase::new();
    let mut store = open_seeded(&temp.database);
    store
        .apply_block_edit(&edit("commit-backup", 0, "sha256:block-initial"))
        .unwrap();
    store.backup_to(&temp.backup).unwrap();

    let backup = OptimizerStore::open(&temp.backup).unwrap();
    assert_eq!(
        backup.get_block("block-1").unwrap().plain_text,
        "harbor signal"
    );
    assert_eq!(backup.list_commits("project-1").unwrap().len(), 2);
    backup.verify_invariants().unwrap();
}

#[test]
fn creates_deterministic_checked_snapshot_and_restores_it_as_a_new_commit() {
    let temp = TempDatabase::new();
    let mut store = open_seeded(&temp.database);
    let snapshot = store
        .create_head_snapshot("snapshot-head", "project-1", "2026-07-14T00:00:30.000Z")
        .unwrap();
    let decoded = store.decode_snapshot_record(&snapshot).unwrap();
    let reencoded = encode_snapshot(&decoded).unwrap();
    assert_eq!(reencoded.payload, snapshot.payload);
    assert_eq!(reencoded.checksum, snapshot.checksum);
    assert_eq!(decoded.commit.id, "commit-initial");

    store
        .apply_block_edit(&edit("commit-after-snapshot", 0, "sha256:block-initial"))
        .unwrap();
    let restored = store
        .restore_snapshot(&RestoreSnapshot {
            snapshot_id: "snapshot-head".into(),
            branch_id: "branch-main".into(),
            new_commit_id: "commit-restore".into(),
            edit_id_prefix: "restore-edit".into(),
            actor_type: "human".into(),
            actor_id: Some("user-local".into()),
            occurred_at: "2026-07-14T00:03:00.000Z".into(),
        })
        .unwrap();
    assert_eq!(restored.previous_head_commit_id, "commit-after-snapshot");
    assert_eq!(restored.restored_root_hash, "sha256:root-initial");
    assert_eq!(restored.changed_blocks, 1);

    let block = store.get_block("block-1").unwrap();
    assert_eq!(block.plain_text, "station platform");
    assert_eq!(block.revision, 2);
    assert_eq!(
        store
            .search_blocks("project-1", "station", 10)
            .unwrap()
            .len(),
        1
    );
    let commits = store.list_commits("project-1").unwrap();
    assert_eq!(commits.len(), 3);
    assert_eq!(commits[2].id, "commit-restore");
    assert_eq!(commits[2].parents, vec!["commit-after-snapshot"]);
    assert_eq!(commits[2].reason, "restore");
    store.verify_invariants().unwrap();
}

#[test]
fn rejects_snapshot_payload_when_checksum_is_tampered() {
    let temp = TempDatabase::new();
    let mut store = open_seeded(&temp.database);
    let mut snapshot = store
        .create_head_snapshot("snapshot-tamper", "project-1", "2026-07-14T00:00:30.000Z")
        .unwrap();
    snapshot.payload[0] ^= 0xff;
    let error = store.decode_snapshot_record(&snapshot).unwrap_err();
    assert!(matches!(error, StoreError::Snapshot(_)));
}

#[test]
fn persists_operation_context_usage_events_and_patch_artifact_atomically() {
    let temp = TempDatabase::new();
    let mut store = open_seeded(&temp.database);
    let stored = store
        .persist_operation_bundle(&operation_bundle("run-1", "context-1", "proposal-1"))
        .unwrap();
    assert_eq!(stored.state, OperationState::Review);
    assert_eq!(stored.context_packet_id.as_deref(), Some("context-1"));
    assert_eq!(stored.usage.as_ref().unwrap().total_tokens, 100);
    assert!(stored.failure.is_none());

    let context = store.get_context_packet("context-1").unwrap();
    assert_eq!(context.operation_intent_id, "intent-run-1");
    let artifact = store.get_operation_artifact("run-1").unwrap();
    assert_eq!(artifact.id, "proposal-1");
    assert_eq!(artifact.kind, OperationArtifactKind::PatchProposal);
    let events = store.list_operation_lifecycle_events("run-1").unwrap();
    assert_eq!(events.len(), 6);
    assert_eq!(events[0].sequence, 1);
    assert_eq!(events.last().unwrap().to_state, OperationState::Review);
    let review = store.get_review_session("proposal-1").unwrap();
    assert_eq!(review.revision, 0);
    assert_eq!(review.status, ReviewSessionStatus::Review);
    store.verify_invariants().unwrap();
}

#[test]
fn rolls_back_every_operation_row_when_artifact_insert_fails_late() {
    let temp = TempDatabase::new();
    let mut store = open_seeded(&temp.database);
    store
        .persist_operation_bundle(&operation_bundle("run-1", "context-1", "proposal-shared"))
        .unwrap();

    let error = store
        .persist_operation_bundle(&operation_bundle("run-2", "context-2", "proposal-shared"))
        .unwrap_err();
    assert!(matches!(error, StoreError::Sqlite(_)));
    assert!(matches!(
        store.get_operation_run("run-2"),
        Err(StoreError::NotFound { .. })
    ));
    assert!(matches!(
        store.get_context_packet("context-2"),
        Err(StoreError::NotFound { .. })
    ));
    assert!(
        store
            .list_operation_lifecycle_events("run-2")
            .unwrap()
            .is_empty()
    );
    store.verify_invariants().unwrap();
}

#[test]
fn persists_failed_runs_without_fabricating_context_or_artifacts() {
    let temp = TempDatabase::new();
    let mut store = open_seeded(&temp.database);
    let bundle = PersistOperationBundle {
        run: NewOperationRun {
            id: "run-failed".into(),
            operation_intent_id: "intent-failed".into(),
            project_id: "project-1".into(),
            base_commit_id: "commit-initial".into(),
            provider_id: "qwen".into(),
            model: "qwen-plus".into(),
            state: OperationState::Failed,
            response_id: None,
            finish_reason: None,
            usage: None,
            failure: Some(OperationFailureRecord {
                code: "CONTEXT_FAILED".into(),
                message: "Context compilation failed".into(),
                retriable: false,
            }),
            started_at: "2026-07-15T00:00:00.000Z".into(),
            updated_at: "2026-07-15T00:00:01.000Z".into(),
        },
        context_packet: None,
        lifecycle_events: vec![
            NewOperationLifecycleEvent {
                from_state: OperationState::Draft,
                to_state: OperationState::Compiling,
                occurred_at: "2026-07-15T00:00:00.000Z".into(),
                reason: None,
            },
            NewOperationLifecycleEvent {
                from_state: OperationState::Compiling,
                to_state: OperationState::Failed,
                occurred_at: "2026-07-15T00:00:01.000Z".into(),
                reason: Some("DomainError".into()),
            },
        ],
        artifact: None,
    };
    let stored = store.persist_operation_bundle(&bundle).unwrap();
    assert_eq!(stored.state, OperationState::Failed);
    assert_eq!(stored.failure.as_ref().unwrap().code, "CONTEXT_FAILED");
    assert!(stored.context_packet_id.is_none());
    assert!(matches!(
        store.get_operation_artifact("run-failed"),
        Err(StoreError::NotFound { .. })
    ));
    store.verify_invariants().unwrap();
}

#[test]
fn persists_findings_as_an_auditable_artifact_without_patch_review_state() {
    let temp = TempDatabase::new();
    let mut store = open_seeded(&temp.database);
    let mut bundle = operation_bundle("run-findings", "context-findings", "findings-1");
    let artifact = bundle.artifact.as_mut().unwrap();
    artifact.kind = OperationArtifactKind::Findings;
    artifact.binding_hash = "sha256:findings".into();
    artifact.payload_json = r#"{"findings":[{"severity":"warning","message":"节奏过快"}]}"#.into();

    store.persist_operation_bundle(&bundle).unwrap();
    let stored = store.get_operation_artifact("run-findings").unwrap();
    assert_eq!(stored.kind, OperationArtifactKind::Findings);
    assert!(matches!(
        store.get_review_session("findings-1"),
        Err(StoreError::NotFound { .. })
    ));
    store.verify_invariants().unwrap();
}

#[test]
fn appends_review_decisions_with_optimistic_concurrency_and_operation_transitions() {
    let temp = TempDatabase::new();
    let mut store = open_seeded(&temp.database);
    store
        .persist_operation_bundle(&operation_bundle(
            "run-review",
            "context-review",
            "proposal-review",
        ))
        .unwrap();

    let ready = store
        .append_review_event(&AppendReviewEvent {
            id: "review-event-1".into(),
            proposal_id: "proposal-review".into(),
            expected_revision: 0,
            expected_status: ReviewSessionStatus::Review,
            kind: ReviewEventKind::Decision,
            next_status: ReviewSessionStatus::Ready,
            hunk_id: Some("hunk-1".into()),
            decision: Some(ReviewDecision::Accepted),
            payload_json: Some(r#"{"source":"inline-review"}"#.into()),
            occurred_at: "2026-07-15T00:02:00.000Z".into(),
        })
        .unwrap();
    assert_eq!(ready.revision, 1);
    assert_eq!(ready.status, ReviewSessionStatus::Ready);
    assert_eq!(
        store.get_operation_run("run-review").unwrap().state,
        OperationState::Review
    );

    let applied = store
        .append_review_event(&AppendReviewEvent {
            id: "review-event-2".into(),
            proposal_id: "proposal-review".into(),
            expected_revision: 1,
            expected_status: ReviewSessionStatus::Ready,
            kind: ReviewEventKind::Apply,
            next_status: ReviewSessionStatus::Applied,
            hunk_id: None,
            decision: None,
            payload_json: Some(r#"{"transactionId":"tx-1"}"#.into()),
            occurred_at: "2026-07-15T00:03:00.000Z".into(),
        })
        .unwrap();
    assert_eq!(applied.revision, 2);
    assert_eq!(applied.status, ReviewSessionStatus::Applied);
    assert_eq!(
        store.get_operation_run("run-review").unwrap().state,
        OperationState::Accepted
    );
    let lifecycle = store.list_operation_lifecycle_events("run-review").unwrap();
    assert_eq!(lifecycle.last().unwrap().from_state, OperationState::Review);
    assert_eq!(lifecycle.last().unwrap().to_state, OperationState::Accepted);
    assert_eq!(
        store.list_review_events("proposal-review").unwrap().len(),
        2
    );

    let stale = store
        .append_review_event(&AppendReviewEvent {
            id: "review-event-stale".into(),
            proposal_id: "proposal-review".into(),
            expected_revision: 1,
            expected_status: ReviewSessionStatus::Ready,
            kind: ReviewEventKind::Apply,
            next_status: ReviewSessionStatus::Applied,
            hunk_id: None,
            decision: None,
            payload_json: None,
            occurred_at: "2026-07-15T00:04:00.000Z".into(),
        })
        .unwrap_err();
    assert!(matches!(stale, StoreError::StateConflict { .. }));
    assert_eq!(
        store.list_review_events("proposal-review").unwrap().len(),
        2
    );
    store.verify_invariants().unwrap();
}

#[test]
fn persists_conflict_rebase_and_rejection_as_a_contiguous_operation_history() {
    let temp = TempDatabase::new();
    let mut store = open_seeded(&temp.database);
    store
        .persist_operation_bundle(&operation_bundle(
            "run-conflicted",
            "context-conflicted",
            "proposal-conflicted",
        ))
        .unwrap();

    let commands = [
        AppendReviewEvent {
            id: "conflict-event".into(),
            proposal_id: "proposal-conflicted".into(),
            expected_revision: 0,
            expected_status: ReviewSessionStatus::Review,
            kind: ReviewEventKind::Conflict,
            next_status: ReviewSessionStatus::Conflicted,
            hunk_id: None,
            decision: None,
            payload_json: Some(r#"{"code":"TARGET_BLOCK_CHANGED"}"#.into()),
            occurred_at: "2026-07-15T00:02:00.000Z".into(),
        },
        AppendReviewEvent {
            id: "rebase-event".into(),
            proposal_id: "proposal-conflicted".into(),
            expected_revision: 1,
            expected_status: ReviewSessionStatus::Conflicted,
            kind: ReviewEventKind::Rebase,
            next_status: ReviewSessionStatus::Review,
            hunk_id: None,
            decision: None,
            payload_json: Some(r#"{"proposalHash":"sha256:rebased"}"#.into()),
            occurred_at: "2026-07-15T00:03:00.000Z".into(),
        },
        AppendReviewEvent {
            id: "reject-event".into(),
            proposal_id: "proposal-conflicted".into(),
            expected_revision: 2,
            expected_status: ReviewSessionStatus::Review,
            kind: ReviewEventKind::Reject,
            next_status: ReviewSessionStatus::Rejected,
            hunk_id: None,
            decision: None,
            payload_json: None,
            occurred_at: "2026-07-15T00:04:00.000Z".into(),
        },
    ];
    for command in commands {
        store.append_review_event(&command).unwrap();
    }

    let run = store.get_operation_run("run-conflicted").unwrap();
    assert_eq!(run.state, OperationState::Rejected);
    let lifecycle = store
        .list_operation_lifecycle_events("run-conflicted")
        .unwrap();
    assert_eq!(lifecycle.len(), 9);
    assert_eq!(lifecycle[6].to_state, OperationState::Conflicted);
    assert_eq!(lifecycle[7].to_state, OperationState::Review);
    assert_eq!(lifecycle[8].to_state, OperationState::Rejected);
    assert_eq!(
        store
            .get_review_session("proposal-conflicted")
            .unwrap()
            .revision,
        3
    );
    store.verify_invariants().unwrap();
}

#[test]
fn immutable_operation_artifacts_reject_out_of_band_tampering() {
    let temp = TempDatabase::new();
    let mut store = open_seeded(&temp.database);
    store
        .persist_operation_bundle(&operation_bundle(
            "run-tamper",
            "context-tamper",
            "proposal-tamper",
        ))
        .unwrap();
    drop(store);

    let connection = rusqlite::Connection::open(&temp.database).unwrap();
    let artifact = connection.execute(
        "UPDATE operation_artifact SET binding_hash = 'tampered' WHERE id = 'proposal-tamper'",
        [],
    );
    assert!(artifact.is_err());
    let context = connection.execute(
        "DELETE FROM context_packet_record WHERE id = 'context-tamper'",
        [],
    );
    assert!(context.is_err());
}
