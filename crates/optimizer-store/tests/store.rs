use std::fs;
use std::path::{Path, PathBuf};
use std::time::{SystemTime, UNIX_EPOCH};

use optimizer_store::{
    AppendReviewEvent, ApplyBlockEdit, ApplyDocumentBatch, CURRENT_SCHEMA_VERSION,
    CreateDocumentWithBlock, CreateKnowledgeItem, CreateReviewCandidateBranch, CreateSnapshot,
    CreateStyleSample, DocumentMutation, MINIMUM_SQLITE_VERSION, ModelUsageRecord,
    NewContextPacket, NewOperationArtifact, NewOperationAttempt, NewOperationLifecycleEvent,
    NewOperationRun, OperationArtifactKind, OperationFailureRecord, OperationState, OptimizerStore,
    PersistOperationBundle, ProjectSeed, PutSummaryRecord, RestoreSnapshot, ReviewDecision,
    ReviewEventKind, ReviewSessionStatus, SeedBlock, SeedDocument, SetKnowledgeItemStatus,
    SetStyleSampleStatus, StoreError, encode_snapshot,
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
        expected_head_commit_id: if expected_revision == 0 {
            "commit-initial".into()
        } else {
            "commit-after-snapshot".into()
        },
        expected_project_revision: 0,
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
        review_event: None,
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
            provider_configuration_id: None,
            provider_endpoint_id: None,
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
        attempts: vec![
            NewOperationAttempt {
                sequence: 1,
                started_at: "2026-07-15T00:00:20.000Z".into(),
                finished_at: "2026-07-15T00:00:21.000Z".into(),
                outcome: "failed".into(),
                response_started: false,
                response_id: None,
                failure_code: Some("rate_limit".into()),
                failure_kind: Some("rate_limit".into()),
                http_status: Some(429),
                remote_request_id: Some("remote-rate-limit".into()),
                retry_after_ms: Some(800),
                retriable: Some(true),
                retry_delay_ms: Some(800),
            },
            NewOperationAttempt {
                sequence: 2,
                started_at: "2026-07-15T00:00:22.000Z".into(),
                finished_at: "2026-07-15T00:00:50.000Z".into(),
                outcome: "succeeded".into(),
                response_started: true,
                response_id: Some(format!("response-{run_id}")),
                failure_code: None,
                failure_kind: None,
                http_status: None,
                remote_request_id: None,
                retry_after_ms: None,
                retriable: None,
                retry_delay_ms: None,
            },
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

fn candidate_operation_bundle() -> PersistOperationBundle {
    let mut bundle = operation_bundle("run-candidate", "context-candidate", "proposal-candidate");
    bundle.artifact.as_mut().unwrap().payload_json = r#"{
      "schemaVersion": 2,
      "id": "proposal-candidate",
      "operationRunId": "run-candidate",
      "baseCommitId": "commit-initial",
      "target": {
        "documentId": "document-1",
        "blockId": "block-1",
        "baseRevision": 0,
        "baseHash": "sha256:block-initial",
        "from": {"blockId": "block-1", "offset": 0},
        "to": {"blockId": "block-1", "offset": 7}
      },
      "hunks": [{
        "id": "candidate-hunk-1",
        "from": {"blockId": "block-1", "offset": 0},
        "to": {"blockId": "block-1", "offset": 7},
        "original": "station",
        "replacement": "harbor",
        "granularity": "token"
      }],
      "summary": "Replace the location",
      "warnings": [],
      "status": "review",
      "createdAt": "2026-07-15T00:01:00.000Z",
      "proposalHash": "sha256:proposal-candidate"
    }"#
    .into();
    bundle
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
fn stores_style_samples_separately_and_archives_with_optimistic_concurrency() {
    let temp = TempDatabase::new();
    let mut store = open_seeded(&temp.database);
    let created = store
        .create_style_sample(&CreateStyleSample {
            id: "style-1".into(),
            project_id: "project-1".into(),
            title: "短句节奏".into(),
            content: "雨停了。她仍然没有回头。".into(),
            content_hash: "sha256:style-1".into(),
            sensitivity: "local_sensitive".into(),
            created_at: "2026-07-15T01:00:00.000Z".into(),
        })
        .unwrap();
    assert_eq!(created.status, "canonical");
    assert_eq!(created.revision, 0);
    assert_eq!(
        store.list_style_samples("project-1").unwrap(),
        vec![created.clone()]
    );

    let archived = store
        .set_style_sample_status(&SetStyleSampleStatus {
            project_id: "project-1".into(),
            id: created.id,
            expected_revision: 0,
            status: "archived".into(),
            updated_at: "2026-07-15T01:01:00.000Z".into(),
        })
        .unwrap();
    assert_eq!(archived.status, "archived");
    assert_eq!(archived.revision, 1);

    let stale = store
        .set_style_sample_status(&SetStyleSampleStatus {
            project_id: "project-1".into(),
            id: archived.id,
            expected_revision: 0,
            status: "canonical".into(),
            updated_at: "2026-07-15T01:02:00.000Z".into(),
        })
        .unwrap_err();
    assert!(matches!(stale, StoreError::StateConflict { .. }));
}

#[test]
fn stores_canonical_facts_and_constraints_with_explicit_policy() {
    let temp = TempDatabase::new();
    let mut store = open_seeded(&temp.database);
    let fact = store
        .create_knowledge_item(&CreateKnowledgeItem {
            id: "fact-1".into(),
            project_id: "project-1".into(),
            kind: "fact".into(),
            title: "主角视觉".into(),
            content: "事实【主角视觉】：主角左眼失明".into(),
            content_hash: "sha256:fact-1".into(),
            authority: "user_confirmed".into(),
            sensitivity: "local_sensitive".into(),
            severity: None,
            created_at: "2026-07-16T01:00:00.000Z".into(),
        })
        .unwrap();
    let constraint = store
        .create_knowledge_item(&CreateKnowledgeItem {
            id: "constraint-1".into(),
            project_id: "project-1".into(),
            kind: "constraint".into(),
            title: "禁止剧透".into(),
            content: "硬约束【禁止剧透】：本章不得揭示凶手身份".into(),
            content_hash: "sha256:constraint-1".into(),
            authority: "user_confirmed".into(),
            sensitivity: "never_send".into(),
            severity: Some("hard".into()),
            created_at: "2026-07-16T01:01:00.000Z".into(),
        })
        .unwrap();
    assert_eq!(fact.status, "canonical");
    assert_eq!(constraint.severity.as_deref(), Some("hard"));
    assert_eq!(store.list_knowledge_items("project-1").unwrap().len(), 2);

    let archived = store
        .set_knowledge_item_status(&SetKnowledgeItemStatus {
            project_id: "project-1".into(),
            id: fact.id,
            expected_revision: 0,
            status: "archived".into(),
            updated_at: "2026-07-16T01:02:00.000Z".into(),
        })
        .unwrap();
    assert_eq!(archived.status, "archived");
    assert_eq!(archived.revision, 1);

    let rejected = store
        .set_knowledge_item_status(&SetKnowledgeItemStatus {
            project_id: "project-1".into(),
            id: constraint.id,
            expected_revision: 0,
            status: "rejected".into(),
            updated_at: "2026-07-16T01:03:00.000Z".into(),
        })
        .unwrap();
    assert_eq!(rejected.status, "rejected");

    let stale = store
        .set_knowledge_item_status(&SetKnowledgeItemStatus {
            project_id: "project-1".into(),
            id: archived.id,
            expected_revision: 0,
            status: "canonical".into(),
            updated_at: "2026-07-16T01:04:00.000Z".into(),
        })
        .unwrap_err();
    assert!(matches!(stale, StoreError::StateConflict { .. }));
}

#[test]
fn creates_a_document_block_and_commit_atomically() {
    let temp = TempDatabase::new();
    let mut store = open_seeded(&temp.database);
    let command = CreateDocumentWithBlock {
        document_id: "document-2".into(),
        document_parent_id: None,
        document_kind: "chapter".into(),
        document_title: "第二章".into(),
        document_order_key: "z-document-2".into(),
        block_id: "block-2".into(),
        block_kind: "paragraph".into(),
        block_order_key: "a0".into(),
        block_content_json: r#"{"type":"paragraph","content":[]}"#.into(),
        block_plain_text: String::new(),
        block_content_hash: "sha256:block-2".into(),
        block_locked: false,
        commit_id: "commit-document-2".into(),
        branch_id: "branch-main".into(),
        expected_head_commit_id: "commit-initial".into(),
        expected_project_revision: 0,
        new_root_hash: "sha256:root-document-2".into(),
        actor_type: "user".into(),
        actor_id: None,
        occurred_at: "2026-07-15T02:00:00.000Z".into(),
    };
    let receipt = store.create_document_with_block(&command).unwrap();
    assert_eq!(receipt.project_revision, 1);
    assert_eq!(store.list_documents("project-1").unwrap().len(), 2);
    assert_eq!(store.list_blocks("project-1").unwrap().len(), 2);
    assert_eq!(
        store.get_project("project-1").unwrap().head_commit_id,
        command.commit_id
    );

    let duplicate_root_order = store.create_document_with_block(&CreateDocumentWithBlock {
        document_id: "document-duplicate-order".into(),
        document_title: "Duplicate order".into(),
        document_order_key: "a0".into(),
        block_id: "block-duplicate-order".into(),
        block_content_hash: "sha256:block-duplicate-order".into(),
        commit_id: "commit-duplicate-order".into(),
        expected_head_commit_id: "commit-document-2".into(),
        expected_project_revision: 1,
        new_root_hash: "sha256:root-duplicate-order".into(),
        ..command.clone()
    });
    assert!(matches!(
        duplicate_root_order,
        Err(StoreError::Validation(_))
    ));
    assert_eq!(store.list_documents("project-1").unwrap().len(), 2);

    let stale = store.create_document_with_block(&CreateDocumentWithBlock {
        document_id: "document-stale".into(),
        block_id: "block-stale".into(),
        commit_id: "commit-stale".into(),
        ..command
    });
    assert!(matches!(stale, Err(StoreError::StateConflict { .. })));
    assert_eq!(store.list_documents("project-1").unwrap().len(), 2);
}

#[test]
fn mutates_document_lifecycle_as_versioned_atomic_batches() {
    let temp = TempDatabase::new();
    let mut store = open_seeded(&temp.database);
    store
        .create_document_with_block(&CreateDocumentWithBlock {
            document_id: "document-2".into(),
            document_parent_id: None,
            document_kind: "chapter".into(),
            document_title: "Chapter two".into(),
            document_order_key: "z-document-2".into(),
            block_id: "block-2".into(),
            block_kind: "paragraph".into(),
            block_order_key: "a0".into(),
            block_content_json: r#"{"type":"paragraph","content":[]}"#.into(),
            block_plain_text: String::new(),
            block_content_hash: "sha256:block-2".into(),
            block_locked: false,
            commit_id: "commit-document-2".into(),
            branch_id: "branch-main".into(),
            expected_head_commit_id: "commit-initial".into(),
            expected_project_revision: 0,
            new_root_hash: "sha256:root-document-2".into(),
            actor_type: "user".into(),
            actor_id: None,
            occurred_at: "2026-07-16T00:00:00.000Z".into(),
        })
        .unwrap();

    let renamed = store
        .apply_document_batch(&ApplyDocumentBatch {
            mutations: vec![DocumentMutation {
                document_id: "document-2".into(),
                expected_revision: 0,
                parent_id: None,
                kind: "chapter".into(),
                title: "Renamed chapter".into(),
                order_key: "z-document-2".into(),
                active: true,
                before_hash: "sha256:document-2-before-rename".into(),
                after_hash: "sha256:document-2-after-rename".into(),
                operation: "rename".into(),
            }],
            commit_id: "commit-document-rename".into(),
            branch_id: "branch-main".into(),
            expected_head_commit_id: "commit-document-2".into(),
            expected_project_revision: 1,
            new_root_hash: "sha256:root-document-rename".into(),
            reason: "document_rename".into(),
            actor_type: "user".into(),
            actor_id: None,
            occurred_at: "2026-07-16T00:01:00.000Z".into(),
        })
        .unwrap();
    assert_eq!(renamed.project_revision, 2);
    assert_eq!(
        store.get_document("project-1", "document-2").unwrap().title,
        "Renamed chapter"
    );

    store
        .apply_document_batch(&ApplyDocumentBatch {
            mutations: vec![
                DocumentMutation {
                    document_id: "document-1".into(),
                    expected_revision: 0,
                    parent_id: None,
                    kind: "chapter".into(),
                    title: seed().documents[0].title.clone(),
                    order_key: "d-00000002".into(),
                    active: true,
                    before_hash: "sha256:document-1-before-reorder".into(),
                    after_hash: "sha256:document-1-after-reorder".into(),
                    operation: "reorder".into(),
                },
                DocumentMutation {
                    document_id: "document-2".into(),
                    expected_revision: 1,
                    parent_id: None,
                    kind: "chapter".into(),
                    title: "Renamed chapter".into(),
                    order_key: "d-00000001".into(),
                    active: true,
                    before_hash: "sha256:document-2-before-reorder".into(),
                    after_hash: "sha256:document-2-after-reorder".into(),
                    operation: "reorder".into(),
                },
            ],
            commit_id: "commit-document-reorder".into(),
            branch_id: "branch-main".into(),
            expected_head_commit_id: "commit-document-rename".into(),
            expected_project_revision: 2,
            new_root_hash: "sha256:root-document-reorder".into(),
            reason: "document_reorder".into(),
            actor_type: "user".into(),
            actor_id: None,
            occurred_at: "2026-07-16T00:02:00.000Z".into(),
        })
        .unwrap();
    assert_eq!(
        store.list_documents("project-1").unwrap()[0].id,
        "document-2"
    );

    store
        .apply_document_batch(&ApplyDocumentBatch {
            mutations: vec![DocumentMutation {
                document_id: "document-2".into(),
                expected_revision: 2,
                parent_id: None,
                kind: "chapter".into(),
                title: "Renamed chapter".into(),
                order_key: "d-00000001".into(),
                active: false,
                before_hash: "sha256:document-2-before-archive".into(),
                after_hash: "sha256:document-2-after-archive".into(),
                operation: "archive".into(),
            }],
            commit_id: "commit-document-archive".into(),
            branch_id: "branch-main".into(),
            expected_head_commit_id: "commit-document-reorder".into(),
            expected_project_revision: 3,
            new_root_hash: "sha256:root-document-archive".into(),
            reason: "document_archive".into(),
            actor_type: "user".into(),
            actor_id: None,
            occurred_at: "2026-07-16T00:03:00.000Z".into(),
        })
        .unwrap();
    assert_eq!(store.list_documents("project-1").unwrap().len(), 1);
    assert_eq!(
        store.list_archived_documents("project-1").unwrap()[0].id,
        "document-2"
    );
    assert_eq!(store.list_blocks("project-1").unwrap().len(), 1);
    let archived_invalidations = store.list_summary_invalidations("project-1", 100).unwrap();
    assert_eq!(archived_invalidations.len(), 3);
    assert!(
        archived_invalidations
            .iter()
            .all(|item| item.scope_id != "document-2" && item.scope_id != "block-2")
    );

    store
        .apply_document_batch(&ApplyDocumentBatch {
            mutations: vec![DocumentMutation {
                document_id: "document-2".into(),
                expected_revision: 3,
                parent_id: None,
                kind: "chapter".into(),
                title: "Renamed chapter".into(),
                order_key: "d-00000001".into(),
                active: true,
                before_hash: "sha256:document-2-before-restore".into(),
                after_hash: "sha256:document-2-after-restore".into(),
                operation: "restore".into(),
            }],
            commit_id: "commit-document-restore".into(),
            branch_id: "branch-main".into(),
            expected_head_commit_id: "commit-document-archive".into(),
            expected_project_revision: 4,
            new_root_hash: "sha256:root-document-restore".into(),
            reason: "document_restore".into(),
            actor_type: "user".into(),
            actor_id: None,
            occurred_at: "2026-07-16T00:04:00.000Z".into(),
        })
        .unwrap();
    assert_eq!(store.list_documents("project-1").unwrap().len(), 2);
    assert!(
        store
            .list_archived_documents("project-1")
            .unwrap()
            .is_empty()
    );
    assert_eq!(store.list_blocks("project-1").unwrap().len(), 2);
    let restored_invalidations = store.list_summary_invalidations("project-1", 100).unwrap();
    assert_eq!(restored_invalidations.len(), 5);
    assert!(restored_invalidations.iter().any(|item| {
        item.scope_type == "block"
            && item.scope_id == "block-2"
            && item.source_commit_id == "commit-document-restore"
    }));
    assert_eq!(store.get_project("project-1").unwrap().revision, 5);
    assert_eq!(
        store
            .list_commits("project-1")
            .unwrap()
            .last()
            .unwrap()
            .reason,
        "document_restore"
    );
    store.verify_invariants().unwrap();
}

#[test]
fn restores_across_document_creation_by_archiving_and_reviving_structure() {
    let temp = TempDatabase::new();
    let mut store = open_seeded(&temp.database);
    let before = store
        .create_head_snapshot(
            "snapshot-before-document",
            "project-1",
            "2026-07-15T02:00:00.000Z",
        )
        .unwrap();
    store
        .create_document_with_block(&CreateDocumentWithBlock {
            document_id: "document-2".into(),
            document_parent_id: None,
            document_kind: "chapter".into(),
            document_title: "第二章".into(),
            document_order_key: "z-document-2".into(),
            block_id: "block-2".into(),
            block_kind: "paragraph".into(),
            block_order_key: "a0".into(),
            block_content_json:
                r#"{"type":"paragraph","content":[{"type":"text","text":"new chapter"}]}"#.into(),
            block_plain_text: "new chapter".into(),
            block_content_hash: "sha256:block-2".into(),
            block_locked: false,
            commit_id: "commit-document-2".into(),
            branch_id: "branch-main".into(),
            expected_head_commit_id: "commit-initial".into(),
            expected_project_revision: 0,
            new_root_hash: "sha256:root-document-2".into(),
            actor_type: "user".into(),
            actor_id: None,
            occurred_at: "2026-07-15T02:01:00.000Z".into(),
        })
        .unwrap();
    let after = store
        .create_head_snapshot(
            "snapshot-after-document",
            "project-1",
            "2026-07-15T02:02:00.000Z",
        )
        .unwrap();

    let back = store
        .restore_snapshot(&RestoreSnapshot {
            snapshot_id: before.id,
            branch_id: "branch-main".into(),
            new_commit_id: "commit-restore-before".into(),
            edit_id_prefix: "edit-restore-before".into(),
            actor_type: "user".into(),
            actor_id: None,
            occurred_at: "2026-07-15T02:03:00.000Z".into(),
        })
        .unwrap();
    assert_eq!(back.changed_blocks, 1);
    assert_eq!(store.list_documents("project-1").unwrap().len(), 1);
    assert_eq!(store.list_blocks("project-1").unwrap().len(), 1);
    assert!(
        store
            .search_blocks("project-1", "new", 10)
            .unwrap()
            .is_empty()
    );

    let forward = store
        .restore_snapshot(&RestoreSnapshot {
            snapshot_id: after.id,
            branch_id: "branch-main".into(),
            new_commit_id: "commit-restore-after".into(),
            edit_id_prefix: "edit-restore-after".into(),
            actor_type: "user".into(),
            actor_id: None,
            occurred_at: "2026-07-15T02:04:00.000Z".into(),
        })
        .unwrap();
    assert_eq!(forward.changed_blocks, 1);
    assert_eq!(store.list_documents("project-1").unwrap().len(), 2);
    assert_eq!(store.list_blocks("project-1").unwrap().len(), 2);
    assert_eq!(
        store.search_blocks("project-1", "new", 10).unwrap().len(),
        1
    );
    assert_eq!(
        store.get_block("block-2").unwrap().plain_text,
        "new chapter"
    );
    store.verify_invariants().unwrap();
}

#[test]
fn restores_versioned_document_metadata_and_archive_state_from_snapshot() {
    let temp = TempDatabase::new();
    let mut store = open_seeded(&temp.database);
    store
        .create_document_with_block(&CreateDocumentWithBlock {
            document_id: "document-2".into(),
            document_parent_id: None,
            document_kind: "chapter".into(),
            document_title: "Chapter two".into(),
            document_order_key: "z-document-2".into(),
            block_id: "block-2".into(),
            block_kind: "paragraph".into(),
            block_order_key: "a0".into(),
            block_content_json: r#"{"type":"paragraph","content":[]}"#.into(),
            block_plain_text: String::new(),
            block_content_hash: "sha256:block-2".into(),
            block_locked: false,
            commit_id: "commit-document-2".into(),
            branch_id: "branch-main".into(),
            expected_head_commit_id: "commit-initial".into(),
            expected_project_revision: 0,
            new_root_hash: "sha256:root-document-2".into(),
            actor_type: "user".into(),
            actor_id: None,
            occurred_at: "2026-07-16T01:00:00.000Z".into(),
        })
        .unwrap();
    let snapshot = store
        .create_head_snapshot(
            "snapshot-document-metadata",
            "project-1",
            "2026-07-16T01:01:00.000Z",
        )
        .unwrap();

    store
        .apply_document_batch(&ApplyDocumentBatch {
            mutations: vec![DocumentMutation {
                document_id: "document-2".into(),
                expected_revision: 0,
                parent_id: None,
                kind: "chapter".into(),
                title: "Renamed chapter".into(),
                order_key: "z-document-2".into(),
                active: true,
                before_hash: "sha256:before-rename".into(),
                after_hash: "sha256:after-rename".into(),
                operation: "rename".into(),
            }],
            commit_id: "commit-rename-after-snapshot".into(),
            branch_id: "branch-main".into(),
            expected_head_commit_id: "commit-document-2".into(),
            expected_project_revision: 1,
            new_root_hash: "sha256:root-renamed".into(),
            reason: "document_rename".into(),
            actor_type: "user".into(),
            actor_id: None,
            occurred_at: "2026-07-16T01:02:00.000Z".into(),
        })
        .unwrap();
    store
        .apply_document_batch(&ApplyDocumentBatch {
            mutations: vec![
                DocumentMutation {
                    document_id: "document-1".into(),
                    expected_revision: 0,
                    parent_id: None,
                    kind: "chapter".into(),
                    title: seed().documents[0].title.clone(),
                    order_key: "d-00000002".into(),
                    active: true,
                    before_hash: "sha256:document-1-before-reorder".into(),
                    after_hash: "sha256:document-1-after-reorder".into(),
                    operation: "reorder".into(),
                },
                DocumentMutation {
                    document_id: "document-2".into(),
                    expected_revision: 1,
                    parent_id: None,
                    kind: "chapter".into(),
                    title: "Renamed chapter".into(),
                    order_key: "d-00000001".into(),
                    active: true,
                    before_hash: "sha256:document-2-before-reorder".into(),
                    after_hash: "sha256:document-2-after-reorder".into(),
                    operation: "reorder".into(),
                },
            ],
            commit_id: "commit-reorder-after-snapshot".into(),
            branch_id: "branch-main".into(),
            expected_head_commit_id: "commit-rename-after-snapshot".into(),
            expected_project_revision: 2,
            new_root_hash: "sha256:root-reordered".into(),
            reason: "document_reorder".into(),
            actor_type: "user".into(),
            actor_id: None,
            occurred_at: "2026-07-16T01:03:00.000Z".into(),
        })
        .unwrap();
    store
        .apply_document_batch(&ApplyDocumentBatch {
            mutations: vec![DocumentMutation {
                document_id: "document-2".into(),
                expected_revision: 2,
                parent_id: None,
                kind: "chapter".into(),
                title: "Renamed chapter".into(),
                order_key: "d-00000001".into(),
                active: false,
                before_hash: "sha256:before-archive".into(),
                after_hash: "sha256:after-archive".into(),
                operation: "archive".into(),
            }],
            commit_id: "commit-archive-after-snapshot".into(),
            branch_id: "branch-main".into(),
            expected_head_commit_id: "commit-reorder-after-snapshot".into(),
            expected_project_revision: 3,
            new_root_hash: "sha256:root-archived".into(),
            reason: "document_archive".into(),
            actor_type: "user".into(),
            actor_id: None,
            occurred_at: "2026-07-16T01:04:00.000Z".into(),
        })
        .unwrap();

    let restored = store
        .restore_snapshot(&RestoreSnapshot {
            snapshot_id: snapshot.id,
            branch_id: "branch-main".into(),
            new_commit_id: "commit-restore-document-metadata".into(),
            edit_id_prefix: "edit-restore-document-metadata".into(),
            actor_type: "user".into(),
            actor_id: None,
            occurred_at: "2026-07-16T01:05:00.000Z".into(),
        })
        .unwrap();

    assert_eq!(restored.restored_root_hash, "sha256:root-document-2");
    let documents = store.list_documents("project-1").unwrap();
    assert_eq!(documents.len(), 2);
    assert_eq!(documents[0].id, "document-1");
    assert_eq!(documents[1].id, "document-2");
    assert_eq!(documents[1].title, "Chapter two");
    assert_eq!(documents[0].revision, 2);
    assert_eq!(documents[1].revision, 4);
    assert!(
        store
            .list_archived_documents("project-1")
            .unwrap()
            .is_empty()
    );
    assert_eq!(store.list_blocks("project-1").unwrap().len(), 2);
    store.verify_invariants().unwrap();
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
    assert_eq!(store.list_documents("project-1").unwrap()[0].revision, 1);
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
fn coalesces_summary_invalidations_and_rejects_stale_summary_completion() {
    let temp = TempDatabase::new();
    let mut store = open_seeded(&temp.database);
    let initial = store.list_summary_invalidations("project-1", 100).unwrap();
    assert_eq!(initial.len(), 3);
    assert!(
        initial.iter().all(|item| {
            item.source_commit_id == "commit-initial" && item.invalidation_count == 1
        })
    );

    let initial_summary = store
        .put_summary_record(&PutSummaryRecord {
            project_id: "project-1".into(),
            scope_type: "block".into(),
            scope_id: "block-1".into(),
            expected_source_commit_id: "commit-initial".into(),
            source_hash: "sha256:block-initial".into(),
            summary: "A station platform.".into(),
            summary_hash: "sha256:summary-initial".into(),
            provider_id: "ollama".into(),
            model: "qwen3:8b".into(),
            generated_at: "2026-07-14T00:00:30.000Z".into(),
        })
        .unwrap();
    assert_eq!(initial_summary.revision, 0);
    assert_eq!(
        store
            .get_ready_summary_record("project-1", "block", "block-1")
            .unwrap()
            .unwrap()
            .summary,
        "A station platform."
    );
    assert_eq!(
        store
            .list_summary_invalidations("project-1", 100)
            .unwrap()
            .len(),
        2
    );

    store
        .apply_block_edit(&edit("commit-summary-edit", 0, "sha256:block-initial"))
        .unwrap();
    let invalidations = store.list_summary_invalidations("project-1", 100).unwrap();
    assert_eq!(invalidations.len(), 3);
    assert!(
        invalidations
            .iter()
            .all(|item| item.source_commit_id == "commit-summary-edit")
    );
    assert_eq!(
        invalidations
            .iter()
            .find(|item| item.scope_type == "project")
            .unwrap()
            .invalidation_count,
        2
    );
    assert_eq!(
        invalidations
            .iter()
            .find(|item| item.scope_type == "block")
            .unwrap()
            .invalidation_count,
        1
    );
    assert!(
        store
            .get_ready_summary_record("project-1", "block", "block-1")
            .unwrap()
            .is_none()
    );

    let stale = store
        .put_summary_record(&PutSummaryRecord {
            project_id: "project-1".into(),
            scope_type: "block".into(),
            scope_id: "block-1".into(),
            expected_source_commit_id: "commit-initial".into(),
            source_hash: "sha256:block-initial".into(),
            summary: "Stale summary".into(),
            summary_hash: "sha256:summary-stale".into(),
            provider_id: "ollama".into(),
            model: "qwen3:8b".into(),
            generated_at: "2026-07-14T00:01:30.000Z".into(),
        })
        .unwrap_err();
    assert!(matches!(stale, StoreError::StateConflict { .. }));
    let refreshed = store
        .put_summary_record(&PutSummaryRecord {
            project_id: "project-1".into(),
            scope_type: "block".into(),
            scope_id: "block-1".into(),
            expected_source_commit_id: "commit-summary-edit".into(),
            source_hash: "sha256:block-commit-summary-edit".into(),
            summary: "A harbor signal.".into(),
            summary_hash: "sha256:summary-refreshed".into(),
            provider_id: "ollama".into(),
            model: "qwen3:8b".into(),
            generated_at: "2026-07-14T00:02:00.000Z".into(),
        })
        .unwrap();
    assert_eq!(refreshed.revision, 1);
    assert_eq!(refreshed.source_commit_id, "commit-summary-edit");
    assert_eq!(
        store
            .get_ready_summary_record("project-1", "block", "block-1")
            .unwrap()
            .unwrap()
            .source_commit_id,
        "commit-summary-edit"
    );
    assert_eq!(
        store
            .list_summary_invalidations("project-1", 100)
            .unwrap()
            .len(),
        2
    );
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
fn stale_project_head_rejects_an_otherwise_current_block_edit() {
    let temp = TempDatabase::new();
    let mut store = open_seeded(&temp.database);
    store
        .apply_block_edit(&edit("commit-first", 0, "sha256:block-initial"))
        .unwrap();
    let mut stale = edit("commit-stale-head", 1, "sha256:block-commit-first");
    stale.expected_head_commit_id = "commit-initial".into();
    stale.expected_project_revision = 1;
    assert!(matches!(
        store.apply_block_edit(&stale),
        Err(StoreError::StateConflict { .. })
    ));
    let block = store.get_block("block-1").unwrap();
    assert_eq!(block.revision, 1);
    assert_eq!(block.content_hash, "sha256:block-commit-first");
    assert_eq!(store.list_commits("project-1").unwrap().len(), 2);
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
    assert_eq!(store.list_documents("project-1").unwrap()[0].revision, 2);
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
    let attempts = store.list_operation_attempts("run-1").unwrap();
    assert_eq!(attempts.len(), 2);
    assert_eq!(attempts[0].failure_kind.as_deref(), Some("rate_limit"));
    assert_eq!(attempts[0].retry_delay_ms, Some(800));
    assert_eq!(attempts[1].outcome, "succeeded");
    let review = store.get_review_session("proposal-1").unwrap();
    assert_eq!(review.revision, 0);
    assert_eq!(review.status, ReviewSessionStatus::Review);
    store.verify_invariants().unwrap();
}

#[test]
fn persists_openai_compatible_endpoint_provenance_and_rejects_unbound_runs() {
    let temp = TempDatabase::new();
    let mut store = open_seeded(&temp.database);
    let endpoint_id = "endpoint-0123456789abcdef0123456789abcdef";
    let configuration_id = "provider-openai-compatible-endpoint-0123456789abcdef0123456789abcdef";

    let mut compatible = operation_bundle(
        "run-compatible",
        "context-compatible",
        "proposal-compatible",
    );
    compatible.run.provider_id = "openai_compatible".into();
    compatible.run.provider_configuration_id = Some(configuration_id.into());
    compatible.run.provider_endpoint_id = Some(endpoint_id.into());
    compatible.run.model = "acme-writer-v1".into();

    let stored = store.persist_operation_bundle(&compatible).unwrap();
    assert_eq!(
        stored.provider_configuration_id.as_deref(),
        Some(configuration_id)
    );
    assert_eq!(stored.provider_endpoint_id.as_deref(), Some(endpoint_id));

    let mut unbound = operation_bundle("run-unbound", "context-unbound", "proposal-unbound");
    unbound.run.provider_id = "openai_compatible".into();
    unbound.run.model = "acme-writer-v1".into();
    let error = store.persist_operation_bundle(&unbound).unwrap_err();
    assert!(matches!(error, StoreError::Validation(_)));

    let mut forged = operation_bundle("run-forged", "context-forged", "proposal-forged");
    forged.run.provider_id = "openai_compatible".into();
    forged.run.provider_configuration_id = Some("provider-forged".into());
    forged.run.provider_endpoint_id = Some(endpoint_id.into());
    forged.run.model = "acme-writer-v1".into();
    let error = store.persist_operation_bundle(&forged).unwrap_err();
    assert!(matches!(error, StoreError::Validation(_)));

    let mut built_in = operation_bundle("run-built-in", "context-built-in", "proposal-built-in");
    built_in.run.provider_endpoint_id = Some(endpoint_id.into());
    let error = store.persist_operation_bundle(&built_in).unwrap_err();
    assert!(matches!(error, StoreError::Validation(_)));
}

#[test]
fn lists_persistent_review_candidates_and_creates_a_snapshot_backed_branch() {
    let temp = TempDatabase::new();
    let mut store = open_seeded(&temp.database);
    store
        .persist_operation_bundle(&candidate_operation_bundle())
        .unwrap();
    store
        .append_review_event(&AppendReviewEvent {
            id: "candidate-decision-1".into(),
            proposal_id: "proposal-candidate".into(),
            expected_revision: 0,
            expected_status: ReviewSessionStatus::Review,
            kind: ReviewEventKind::Decision,
            next_status: ReviewSessionStatus::Ready,
            hunk_id: Some("candidate-hunk-1".into()),
            decision: Some(ReviewDecision::Accepted),
            payload_json: None,
            occurred_at: "2026-07-15T00:02:00.000Z".into(),
        })
        .unwrap();

    let candidates = store.list_review_candidates("project-1", 100).unwrap();
    assert_eq!(candidates.len(), 1);
    assert_eq!(candidates[0].proposal_id, "proposal-candidate");
    assert_eq!(candidates[0].target_block_id, "block-1");
    assert_eq!(candidates[0].hunk_count, 1);
    assert_eq!(
        candidates[0].summary.as_deref(),
        Some("Replace the location")
    );
    assert_eq!(candidates[0].status, ReviewSessionStatus::Ready);
    assert!(candidates[0].candidate_branch.is_none());

    let branch = store
        .create_review_candidate_branch(&CreateReviewCandidateBranch {
            proposal_id: "proposal-candidate".into(),
            expected_review_revision: 1,
            project_id: "project-1".into(),
            expected_project_head_commit_id: "commit-initial".into(),
            branch_id: "branch-candidate".into(),
            branch_name: "AI 候选：港口".into(),
            commit_id: "commit-candidate".into(),
            snapshot_id: "snapshot-candidate".into(),
            target_document_id: "document-1".into(),
            target_block_id: "block-1".into(),
            expected_block_revision: 0,
            expected_block_hash: "sha256:block-initial".into(),
            new_content_json: r#"{"type":"paragraph","text":"harbor platform"}"#.into(),
            new_plain_text: "harbor platform".into(),
            new_content_hash: "sha256:block-candidate".into(),
            new_root_hash: "sha256:root-candidate".into(),
            occurred_at: "2026-07-15T00:03:00.000Z".into(),
        })
        .unwrap();
    assert_eq!(branch.branch_id, "branch-candidate");
    assert_eq!(
        store.get_project("project-1").unwrap().head_commit_id,
        "commit-initial"
    );
    assert_eq!(
        store.get_block("block-1").unwrap().plain_text,
        "station platform"
    );
    assert_eq!(
        store
            .get_review_session("proposal-candidate")
            .unwrap()
            .status,
        ReviewSessionStatus::Ready
    );
    assert_eq!(
        store.get_branch("branch-candidate").unwrap().head_commit_id,
        "commit-candidate"
    );
    let snapshot_record = store.get_snapshot("snapshot-candidate").unwrap();
    let snapshot = store.decode_snapshot_record(&snapshot_record).unwrap();
    assert_eq!(snapshot.commit.id, "commit-candidate");
    assert_eq!(
        snapshot
            .blocks
            .iter()
            .find(|block| block.id == "block-1")
            .unwrap()
            .plain_text,
        "harbor platform"
    );
    let candidate = store
        .get_review_candidate_summary("proposal-candidate")
        .unwrap();
    assert_eq!(
        candidate
            .candidate_branch
            .as_ref()
            .map(|value| value.branch_name.as_str()),
        Some("AI 候选：港口")
    );
    assert!(matches!(
        store.create_review_candidate_branch(&CreateReviewCandidateBranch {
            proposal_id: "proposal-candidate".into(),
            expected_review_revision: 1,
            project_id: "project-1".into(),
            expected_project_head_commit_id: "commit-initial".into(),
            branch_id: "branch-candidate-2".into(),
            branch_name: "duplicate".into(),
            commit_id: "commit-candidate-2".into(),
            snapshot_id: "snapshot-candidate-2".into(),
            target_document_id: "document-1".into(),
            target_block_id: "block-1".into(),
            expected_block_revision: 0,
            expected_block_hash: "sha256:block-initial".into(),
            new_content_json: r#"{"type":"paragraph","text":"harbor platform"}"#.into(),
            new_plain_text: "harbor platform".into(),
            new_content_hash: "sha256:block-candidate".into(),
            new_root_hash: "sha256:root-candidate".into(),
            occurred_at: "2026-07-15T00:04:00.000Z".into(),
        }),
        Err(StoreError::Validation(_))
    ));
    store.verify_invariants().unwrap();
    drop(store);

    let reopened = OptimizerStore::open(&temp.database).unwrap();
    assert_eq!(
        reopened
            .list_review_candidates("project-1", 100)
            .unwrap()
            .first()
            .and_then(|value| value.candidate_branch.as_ref())
            .map(|value| value.commit_id.as_str()),
        Some("commit-candidate")
    );
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
            provider_configuration_id: None,
            provider_endpoint_id: None,
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
        attempts: Vec::new(),
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
    let attempt = connection.execute(
        "UPDATE operation_attempt SET retry_delay_ms = 1 WHERE run_id = 'run-tamper' AND sequence = 1",
        [],
    );
    assert!(attempt.is_err());
    let impossible_attempt = connection.execute(
        "INSERT INTO operation_attempt(
           run_id, sequence, started_at, finished_at, outcome, response_started,
           response_id, failure_code, retriable
         ) VALUES (
           'run-tamper', 3, '2026-07-15T00:00:51Z', '2026-07-15T00:00:52Z',
           'failed', 0, 'response-without-start', 'PROVIDER_FAILED', 0
         )",
        [],
    );
    assert!(impossible_attempt.is_err());
}
