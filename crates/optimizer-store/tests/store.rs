use std::fs;
use std::path::{Path, PathBuf};
use std::time::{SystemTime, UNIX_EPOCH};

use optimizer_store::{
    ApplyBlockEdit, CURRENT_SCHEMA_VERSION, CreateSnapshot, MINIMUM_SQLITE_VERSION, OptimizerStore,
    ProjectSeed, RestoreSnapshot, SeedBlock, SeedDocument, StoreError, encode_snapshot,
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
