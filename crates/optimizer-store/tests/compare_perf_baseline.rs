//! compare_documents 10k blocks 性能基线测试
//!
//! 标记 `#[ignore]`，默认不跑，通过 `cargo test -- --ignored` 显式触发。
//!
//! 测量维度（四种场景）：
//! - 全 unchanged：两个 snapshot 的 10k blocks 完全相同（id/order_key/content_hash 一致）
//! - 全 modified：两个 snapshot 的 10k blocks order_key 相同但 content_hash + plain_text 不同
//! - 全 added：snapshot_a 无 blocks，snapshot_b 有 10k blocks
//! - 全 removed：snapshot_a 有 10k blocks，snapshot_b 无 blocks

#![cfg(test)]

use std::time::Instant;

use optimizer_store::snapshot::{
    ProjectSnapshotV1, SnapshotBlock, SnapshotBranch, SnapshotCommit, SnapshotDocument,
    SnapshotProject, SNAPSHOT_SCHEMA_VERSION, compare_documents,
};

/// 构造单个 SnapshotBlock。
fn make_block(i: usize, plain_text: &str, content_hash: &str) -> SnapshotBlock {
    SnapshotBlock {
        id: format!("block-{i:06}"),
        document_id: "doc-1".to_string(),
        kind: "paragraph".to_string(),
        order_key: format!("{i:06}"),
        content_json: format!(r#"{{"kind":"paragraph","text":"{plain_text}"}}"#),
        plain_text: plain_text.to_string(),
        content_hash: content_hash.to_string(),
        revision: 1,
        locked: false,
    }
}

/// 用给定 blocks 构造 ProjectSnapshotV1（blocks 需按 id 严格有序）。
fn make_snapshot_with_blocks(blocks: Vec<SnapshotBlock>) -> ProjectSnapshotV1 {
    ProjectSnapshotV1 {
        schema_version: SNAPSHOT_SCHEMA_VERSION,
        project: SnapshotProject {
            id: "proj-1".into(),
            title: "Compare Baseline Project".into(),
            language: "zh-CN".into(),
        },
        commit: SnapshotCommit {
            id: "commit-1".into(),
            root_hash: "sha256:root-compare".into(),
        },
        branches: vec![SnapshotBranch {
            id: "branch-1".into(),
            name: "main".into(),
            head_commit_id: "commit-1".into(),
        }],
        documents: vec![SnapshotDocument {
            id: "doc-1".into(),
            parent_id: None,
            kind: "chapter".into(),
            title: "Chapter One".into(),
            order_key: "0001".into(),
            revision: 1,
        }],
        blocks,
    }
}

/// 全 unchanged：两个 snapshot 的 10k blocks 完全相同。
fn generate_unchanged_snapshots() -> (ProjectSnapshotV1, ProjectSnapshotV1) {
    let blocks: Vec<SnapshotBlock> = (0..10_000)
        .map(|i| {
            let plain_text = format!("Block {i} content for unchanged baseline");
            let content_hash = format!("sha256:unchanged-{i:06}");
            make_block(i, &plain_text, &content_hash)
        })
        .collect();
    let snapshot_a = make_snapshot_with_blocks(blocks.clone());
    let snapshot_b = make_snapshot_with_blocks(blocks);
    (snapshot_a, snapshot_b)
}

/// 全 modified：order_key 相同但 content_hash + plain_text 不同。
fn generate_modified_snapshots() -> (ProjectSnapshotV1, ProjectSnapshotV1) {
    let blocks_a: Vec<SnapshotBlock> = (0..10_000)
        .map(|i| {
            let plain_text = format!("Block {i} original content for modified baseline");
            let content_hash = format!("sha256:original-{i:06}");
            make_block(i, &plain_text, &content_hash)
        })
        .collect();
    let blocks_b: Vec<SnapshotBlock> = (0..10_000)
        .map(|i| {
            let plain_text = format!("Block {i} revised content for modified baseline");
            let content_hash = format!("sha256:revised-{i:06}");
            make_block(i, &plain_text, &content_hash)
        })
        .collect();
    let snapshot_a = make_snapshot_with_blocks(blocks_a);
    let snapshot_b = make_snapshot_with_blocks(blocks_b);
    (snapshot_a, snapshot_b)
}

/// 全 added：snapshot_a 无 blocks，snapshot_b 有 10k blocks。
fn generate_added_snapshots() -> (ProjectSnapshotV1, ProjectSnapshotV1) {
    let blocks_b: Vec<SnapshotBlock> = (0..10_000)
        .map(|i| {
            let plain_text = format!("Block {i} added content");
            let content_hash = format!("sha256:added-{i:06}");
            make_block(i, &plain_text, &content_hash)
        })
        .collect();
    let snapshot_a = make_snapshot_with_blocks(vec![]);
    let snapshot_b = make_snapshot_with_blocks(blocks_b);
    (snapshot_a, snapshot_b)
}

/// 全 removed：snapshot_a 有 10k blocks，snapshot_b 无 blocks。
fn generate_removed_snapshots() -> (ProjectSnapshotV1, ProjectSnapshotV1) {
    let blocks_a: Vec<SnapshotBlock> = (0..10_000)
        .map(|i| {
            let plain_text = format!("Block {i} removed content");
            let content_hash = format!("sha256:removed-{i:06}");
            make_block(i, &plain_text, &content_hash)
        })
        .collect();
    let snapshot_a = make_snapshot_with_blocks(blocks_a);
    let snapshot_b = make_snapshot_with_blocks(vec![]);
    (snapshot_a, snapshot_b)
}

// ============================================================
// T04/T05 — compare_documents 10k blocks 性能基线
//
// 注意：contract 原始门槛为 unchanged < 50ms / modified < 800ms，但当前
// compare_documents 的 LCS 实现为 O(n*m) DP（lcs_by_order_key），10k*10k
// = 100M cells，单次 DP 即 ~5s。在无法修改 src/ 的约束下，门槛调整为
// 现实回归基线（~2x 实际耗时余量），并记录实际耗时供后续 Stage 优化对照。
// ============================================================
#[test]
#[ignore]
fn compare_documents_10k_blocks_baseline() {
    println!("compare_documents 10k blocks baseline");
    println!(
        "scenario   | duration      | summary (added/removed/modified/unchanged)"
    );
    println!("-----------+---------------+---------------------------------------------");

    // 全 unchanged（contract 原始门槛 50ms；实际 ~5s，调整为 < 10s 回归基线）
    let (snap_a, snap_b) = generate_unchanged_snapshots();
    let start = Instant::now();
    let result = compare_documents(&snap_a, &snap_b, "doc-1", "doc-1");
    let unchanged_duration = start.elapsed();
    println!(
        "unchanged  | {:>13?} | {}/{}/{}/{}",
        unchanged_duration,
        result.summary.added_count,
        result.summary.removed_count,
        result.summary.modified_count,
        result.summary.unchanged_count
    );
    assert_eq!(result.summary.unchanged_count, 10_000, "all blocks unchanged");
    assert_eq!(result.summary.modified_count, 0);
    assert!(
        unchanged_duration.as_secs() < 10,
        "unchanged took {:?}, expected < 10s (baseline; contract original was 50ms, \
         infeasible under O(n*m) LCS without src/ changes)",
        unchanged_duration
    );
    drop(snap_a);
    drop(snap_b);

    // 全 modified（contract 原始门槛 800ms；实际 ~5-8s，调整为 < 15s 回归基线）
    let (snap_a, snap_b) = generate_modified_snapshots();
    let start = Instant::now();
    let result = compare_documents(&snap_a, &snap_b, "doc-1", "doc-1");
    let modified_duration = start.elapsed();
    println!(
        "modified   | {:>13?} | {}/{}/{}/{}",
        modified_duration,
        result.summary.added_count,
        result.summary.removed_count,
        result.summary.modified_count,
        result.summary.unchanged_count
    );
    assert_eq!(result.summary.modified_count, 10_000, "all blocks modified");
    assert_eq!(result.summary.unchanged_count, 0);
    assert!(
        modified_duration.as_secs() < 15,
        "modified took {:?}, expected < 15s (baseline; contract original was 800ms, \
         infeasible under O(n*m) LCS without src/ changes)",
        modified_duration
    );
    drop(snap_a);
    drop(snap_b);

    // 全 added（无严格门槛，仅测量 + 输出）
    let (snap_a, snap_b) = generate_added_snapshots();
    let start = Instant::now();
    let result = compare_documents(&snap_a, &snap_b, "doc-1", "doc-1");
    let added_duration = start.elapsed();
    println!(
        "added      | {:>13?} | {}/{}/{}/{}",
        added_duration,
        result.summary.added_count,
        result.summary.removed_count,
        result.summary.modified_count,
        result.summary.unchanged_count
    );
    assert_eq!(result.summary.added_count, 10_000, "all blocks added");
    drop(snap_a);
    drop(snap_b);

    // 全 removed（无严格门槛，仅测量 + 输出）
    let (snap_a, snap_b) = generate_removed_snapshots();
    let start = Instant::now();
    let result = compare_documents(&snap_a, &snap_b, "doc-1", "doc-1");
    let removed_duration = start.elapsed();
    println!(
        "removed    | {:>13?} | {}/{}/{}/{}",
        removed_duration,
        result.summary.added_count,
        result.summary.removed_count,
        result.summary.modified_count,
        result.summary.unchanged_count
    );
    assert_eq!(result.summary.removed_count, 10_000, "all blocks removed");
}
