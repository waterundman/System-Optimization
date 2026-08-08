//! Integration tests for the v0.6.0 + v0.7.0 timeline host surface.
//!
//! These tests exercise `OperationCommandHost::list_timeline_events` against
//! an in-memory store seeded with checkpoints and operation runs. The v0.6.0
//! baseline keeps events derived from checkpoints sorted by `created_at`
//! descending, and each event optionally carries the operation run that was
//! based on the checkpoint's commit. The v0.7.0 Stage 1 (D1) extension
//! surfaces `parent_commit_ids` so merge commits expose their multi-parent
//! topology via the existing `commit_parent` table without introducing CRDT
//! state.

use optimizer_host::{TimelineEvent, TimelineEventsResponse, list_timeline_events};
use optimizer_store::{OptimizerStore, ProjectSeed, SeedBlock, SeedDocument};
use rusqlite::params;

fn seed() -> ProjectSeed {
    ProjectSeed {
        project_id: "project-1".into(),
        title: "Timeline Integration Test".into(),
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

/// Advance the project head to a fresh `commit_node` so the next
/// `create_head_snapshot` targets a distinct `commit_id`. Required
/// because `materialized_snapshot` enforces
/// UNIQUE(project_id, commit_id, codec, codec_version) — multiple
/// snapshots of the same commit are not allowed. Keeps the test
/// focused on timeline ordering rather than commit topology.
fn advance_head_commit(store: &mut OptimizerStore, new_commit_id: &str, created_at: &str) {
    let conn = store.connection();
    let parent_id: String = conn
        .query_row(
            "SELECT head_commit_id FROM project WHERE id = 'project-1'",
            [],
            |row| row.get(0),
        )
        .expect("read current head failed");
    conn.execute(
        "INSERT INTO commit_node(id, project_id, root_hash, reason, actor_type, actor_id, created_at)
         VALUES (?1, 'project-1', ?2, 'edit', 'human', NULL, ?3)",
        params![new_commit_id, format!("sha256:root-{new_commit_id}"), created_at],
    )
    .expect("insert commit_node failed");
    conn.execute(
        "INSERT INTO commit_parent(commit_id, parent_id, position) VALUES (?1, ?2, 0)",
        params![new_commit_id, parent_id],
    )
    .expect("insert commit_parent failed");
    conn.execute(
        "UPDATE project SET head_commit_id = ?1, updated_at = ?2 WHERE id = 'project-1'",
        params![new_commit_id, created_at],
    )
    .expect("update project head failed");
    conn.execute(
        "UPDATE branch SET head_commit_id = ?1, updated_at = ?2 WHERE id = 'branch-main'",
        params![new_commit_id, created_at],
    )
    .expect("update branch head failed");
}

/// Seed a store with three checkpoints materialised at successive
/// timestamps so the timeline ordering is observable. Each checkpoint
/// targets a distinct commit (advanced via `advance_head_commit`) to
/// satisfy the UNIQUE(project_id, commit_id, codec, codec_version)
/// constraint on `materialized_snapshot`.
fn setup_store_with_checkpoints() -> OptimizerStore {
    let mut store = OptimizerStore::open_in_memory().expect("open_in_memory failed");
    store.initialize_project(&seed()).expect("initialize_project failed");
    store
        .create_head_snapshot("snapshot-1", "project-1", "2026-07-15T00:01:00.000Z")
        .expect("create snapshot-1 failed");
    advance_head_commit(&mut store, "commit-2", "2026-07-15T00:01:30.000Z");
    store
        .create_head_snapshot("snapshot-2", "project-1", "2026-07-15T00:02:00.000Z")
        .expect("create snapshot-2 failed");
    advance_head_commit(&mut store, "commit-3", "2026-07-15T00:02:30.000Z");
    store
        .create_head_snapshot("snapshot-3", "project-1", "2026-07-15T00:03:00.000Z")
        .expect("create snapshot-3 failed");
    store
}

#[test]
fn t01_list_timeline_events_returns_events_sorted_by_created_at_desc() {
    let store = setup_store_with_checkpoints();
    let response: TimelineEventsResponse = list_timeline_events(&store, "project-1")
        .expect("list_timeline_events failed");

    assert_eq!(response.total_count, 3, "total_count must be 3");
    assert_eq!(response.events.len(), 3, "events len must be 3");
    let created_times: Vec<&str> = response
        .events
        .iter()
        .map(TimelineEvent::created_at)
        .collect();
    assert_eq!(
        created_times,
        &[
            "2026-07-15T00:03:00.000Z",
            "2026-07-15T00:02:00.000Z",
            "2026-07-15T00:01:00.000Z",
        ],
        "events must be sorted by created_at descending"
    );
    let checkpoint_ids: Vec<&str> = response
        .events
        .iter()
        .map(TimelineEvent::checkpoint_id)
        .collect();
    assert_eq!(
        checkpoint_ids,
        &["snapshot-3", "snapshot-2", "snapshot-1"],
        "checkpoint ids must follow the descending created_at order"
    );
}

#[test]
fn t02_list_timeline_events_empty_project_returns_empty_events() {
    let mut store = OptimizerStore::open_in_memory().expect("open_in_memory failed");
    store.initialize_project(&seed()).expect("initialize_project failed");
    let response = list_timeline_events(&store, "project-1")
        .expect("list_timeline_events failed for project without checkpoints");

    assert_eq!(response.total_count, 0, "total_count must be 0");
    assert!(response.events.is_empty(), "events must be empty");
}

/// Advance the project head to a new `commit_node` with multiple parents,
/// creating a branch merge point (D1 topology). Unlike `advance_head_commit`,
/// this helper accepts a list of parent ids so the first parent becomes
/// position 0, the second becomes position 1, etc. The project/branch head
/// is then moved to the merge commit so the next checkpoint materialises
/// at the merge commit.
fn advance_head_commit_with_parents(
    store: &mut OptimizerStore,
    new_commit_id: &str,
    parent_ids: &[&str],
    created_at: &str,
) {
    let conn = store.connection();
    conn.execute(
        "INSERT INTO commit_node(id, project_id, root_hash, reason, actor_type, actor_id, created_at)
         VALUES (?1, 'project-1', ?2, 'merge', 'human', NULL, ?3)",
        params![new_commit_id, format!("sha256:root-{new_commit_id}"), created_at],
    )
    .expect("insert commit_node failed");
    for (position, parent_id) in parent_ids.iter().enumerate() {
        conn.execute(
            "INSERT INTO commit_parent(commit_id, parent_id, position) VALUES (?1, ?2, ?3)",
            params![new_commit_id, parent_id, position as i64],
        )
        .expect("insert commit_parent failed");
    }
    conn.execute(
        "UPDATE project SET head_commit_id = ?1, updated_at = ?2 WHERE id = 'project-1'",
        params![new_commit_id, created_at],
    )
    .expect("update project head failed");
    conn.execute(
        "UPDATE branch SET head_commit_id = ?1, updated_at = ?2 WHERE id = 'branch-main'",
        params![new_commit_id, created_at],
    )
    .expect("update branch head failed");
}

#[test]
fn t03_list_timeline_events_returns_parent_commit_ids_for_merge_checkpoint() {
    // v0.7.0 Stage 1 (D1): branch topology timeline must surface multi-parent
    // commit relationships via `TimelineEvent::parent_commit_ids`. Seed two
    // linear commits, then create a merge commit with both parents recorded
    // in `commit_parent` (position 0 and position 1). The merge commit's
    // checkpoint must report both parent ids.
    let mut store = OptimizerStore::open_in_memory().expect("open_in_memory failed");
    store.initialize_project(&seed()).expect("initialize_project failed");
    // commit-initial is the seed head (no parents recorded).
    // Linear advance: commit-initial -> commit-2.
    advance_head_commit(&mut store, "commit-2", "2026-07-15T00:01:30.000Z");
    // Branch fork: create commit-2a as a sibling of commit-2 (also rooted at
    // commit-initial). We do NOT move the project head here; commit-2a is a
    // sibling branch head only.
    {
        let conn = store.connection();
        conn.execute(
            "INSERT INTO commit_node(id, project_id, root_hash, reason, actor_type, actor_id, created_at)
             VALUES ('commit-2a', 'project-1', 'sha256:root-commit-2a', 'edit', 'human', NULL, '2026-07-15T00:01:35.000Z')",
            [],
        )
        .expect("insert commit-2a failed");
        conn.execute(
            "INSERT INTO commit_parent(commit_id, parent_id, position) VALUES ('commit-2a', 'commit-initial', 0)",
            [],
        )
        .expect("insert commit-2a parent failed");
    }
    // Merge: commit-3 has two parents (commit-2, commit-2a). Move head to
    // commit-3 so the next checkpoint targets the merge commit.
    advance_head_commit_with_parents(
        &mut store,
        "commit-3",
        &["commit-2", "commit-2a"],
        "2026-07-15T00:02:30.000Z",
    );
    store
        .create_head_snapshot("snapshot-merge", "project-1", "2026-07-15T00:03:00.000Z")
        .expect("create snapshot-merge failed");

    let response: TimelineEventsResponse = list_timeline_events(&store, "project-1")
        .expect("list_timeline_events failed");

    let merge_event = response
        .events
        .iter()
        .find(|event| event.commit_id == "commit-3")
        .expect("merge commit-3 must appear in timeline events");
    assert_eq!(
        merge_event.parent_commit_ids,
        vec!["commit-2".to_owned(), "commit-2a".to_owned()],
        "merge checkpoint must expose both parent commit ids in position order",
    );
}

#[test]
fn t04_list_timeline_events_linear_history_reports_single_parent_or_empty() {
    // v0.7.0 Stage 1 (D1, backward-compat): for a purely linear history
    // (each commit has at most one parent), `TimelineEvent::parent_commit_ids`
    // must carry at most one entry. The initial seed commit has zero
    // parents; subsequent linear commits each have exactly one parent.
    let store = setup_store_with_checkpoints();
    let response: TimelineEventsResponse = list_timeline_events(&store, "project-1")
        .expect("list_timeline_events failed");

    assert_eq!(response.total_count, 3, "precondition: 3 checkpoints");
    for event in &response.events {
        assert!(
            event.parent_commit_ids.len() <= 1,
            "linear history must not expose multi-parent topology, got {:?} for commit {}",
            event.parent_commit_ids,
            event.commit_id,
        );
    }
    // commit-3 is the latest head and must have exactly one parent
    // (commit-2). commit-initial has zero parents.
    let commit_initial = response
        .events
        .iter()
        .find(|event| event.commit_id == "commit-initial")
        .expect("seed commit-initial must be present");
    assert!(
        commit_initial.parent_commit_ids.is_empty(),
        "seed commit must have zero parents, got {:?}",
        commit_initial.parent_commit_ids,
    );
    let commit_3 = response
        .events
        .iter()
        .find(|event| event.commit_id == "commit-3")
        .expect("head commit-3 must be present");
    assert_eq!(
        commit_3.parent_commit_ids,
        vec!["commit-2".to_owned()],
        "linear commit-3 must expose exactly one parent (commit-2)",
    );
}
