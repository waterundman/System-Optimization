use optimizer_store::{
    BlockDiffEntry, BlockDiffKind, DiffSummary, DocumentDiffResult, ProjectSnapshotV1, SNAPSHOT_CODEC,
    SNAPSHOT_CODEC_VERSION, SNAPSHOT_SCHEMA_VERSION, SnapshotBlock, SnapshotBranch, SnapshotCommit,
    SnapshotDocument, SnapshotProject, TextDiffOp, checksum, compare_documents, lcs_by_order_key,
};

fn make_block(
    id: &str,
    document_id: &str,
    order_key: &str,
    plain_text: &str,
    content_hash: &str,
) -> SnapshotBlock {
    SnapshotBlock {
        id: id.into(),
        document_id: document_id.into(),
        kind: "paragraph".into(),
        order_key: order_key.into(),
        content_json: format!(r#"{{"kind":"paragraph","text":"{plain_text}"}}"#),
        plain_text: plain_text.into(),
        content_hash: content_hash.into(),
        revision: 1,
        locked: false,
    }
}

fn make_snapshot(blocks: Vec<SnapshotBlock>) -> ProjectSnapshotV1 {
    let mut sorted_blocks = blocks;
    sorted_blocks.sort_by(|a, b| a.id.cmp(&b.id));
    ProjectSnapshotV1 {
        schema_version: SNAPSHOT_SCHEMA_VERSION,
        project: SnapshotProject {
            id: "project-1".into(),
            title: "Test Project".into(),
            language: "zh-CN".into(),
        },
        commit: SnapshotCommit {
            id: "commit-1".into(),
            root_hash: "sha256:abc".into(),
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
        blocks: sorted_blocks,
    }
}

fn hash_of(text: &str) -> String {
    checksum(text.as_bytes())
}

#[test]
fn t01_compare_documents_same_document_returns_all_unchanged() {
    let block_a = make_block("block-a", "doc-1", "0001", "hello", "sha256:a");
    let block_b = make_block("block-b", "doc-1", "0002", "world", "sha256:b");
    let snapshot_a = make_snapshot(vec![block_a.clone(), block_b.clone()]);
    let snapshot_b = make_snapshot(vec![block_a, block_b]);
    let result = compare_documents(&snapshot_a, &snapshot_b, "doc-1", "doc-1");
    assert_eq!(result.document_id_a, "doc-1");
    assert_eq!(result.document_id_b, "doc-1");
    assert_eq!(result.blocks.len(), 2);
    assert_eq!(result.summary.unchanged_count, 2);
    assert_eq!(result.summary.added_count, 0);
    assert_eq!(result.summary.removed_count, 0);
    assert_eq!(result.summary.modified_count, 0);
    for entry in &result.blocks {
        assert_eq!(entry.kind, BlockDiffKind::Unchanged);
        assert!(entry.text_diff.is_none());
    }
}

#[test]
fn t02_compare_documents_pure_added_blocks() {
    let snapshot_a = make_snapshot(vec![]);
    let snapshot_b = make_snapshot(vec![
        make_block("block-b1", "doc-1", "0001", "first", "sha256:b1"),
        make_block("block-b2", "doc-1", "0002", "second", "sha256:b2"),
    ]);
    let result = compare_documents(&snapshot_a, &snapshot_b, "doc-1", "doc-1");
    assert_eq!(result.blocks.len(), 2);
    assert_eq!(result.summary.added_count, 2);
    for entry in &result.blocks {
        assert_eq!(entry.kind, BlockDiffKind::Added);
        assert!(entry.block_id_a.is_none());
        assert!(entry.block_id_b.is_some());
        assert!(entry.text_diff.is_none());
    }
}

#[test]
fn t03_compare_documents_pure_removed_blocks() {
    let snapshot_a = make_snapshot(vec![
        make_block("block-a1", "doc-1", "0001", "first", "sha256:a1"),
        make_block("block-a2", "doc-1", "0002", "second", "sha256:a2"),
    ]);
    let snapshot_b = make_snapshot(vec![]);
    let result = compare_documents(&snapshot_a, &snapshot_b, "doc-1", "doc-1");
    assert_eq!(result.blocks.len(), 2);
    assert_eq!(result.summary.removed_count, 2);
    for entry in &result.blocks {
        assert_eq!(entry.kind, BlockDiffKind::Removed);
        assert!(entry.block_id_a.is_some());
        assert!(entry.block_id_b.is_none());
        assert!(entry.text_diff.is_none());
    }
}

#[test]
fn t04_compare_documents_modified_block_returns_text_diff() {
    let snapshot_a = make_snapshot(vec![make_block(
        "block-a",
        "doc-1",
        "0001",
        "hello world",
        "sha256:a",
    )]);
    let snapshot_b = make_snapshot(vec![make_block(
        "block-b",
        "doc-1",
        "0001",
        "hello there",
        "sha256:b",
    )]);
    let result = compare_documents(&snapshot_a, &snapshot_b, "doc-1", "doc-1");
    assert_eq!(result.blocks.len(), 1);
    let entry = &result.blocks[0];
    assert_eq!(entry.kind, BlockDiffKind::Modified);
    assert_eq!(entry.block_id_a.as_deref(), Some("block-a"));
    assert_eq!(entry.block_id_b.as_deref(), Some("block-b"));
    let text_diff = entry.text_diff.as_ref().expect("text_diff is set for Modified");
    assert!(!text_diff.is_empty());
    let reconstructed_a: String = text_diff
        .iter()
        .filter_map(|op| match op {
            TextDiffOp::Equal(s) | TextDiffOp::Delete(s) => Some(s.as_str()),
            _ => None,
        })
        .collect();
    let reconstructed_b: String = text_diff
        .iter()
        .filter_map(|op| match op {
            TextDiffOp::Equal(s) | TextDiffOp::Insert(s) => Some(s.as_str()),
            _ => None,
        })
        .collect();
    assert_eq!(reconstructed_a, "hello world");
    assert_eq!(reconstructed_b, "hello there");
    assert_eq!(result.summary.modified_count, 1);
}

#[test]
fn t05_compare_documents_lcs_by_order_key_aligns_unsorted_blocks() {
    let snapshot_a = make_snapshot(vec![
        make_block("block-a1", "doc-1", "0001", "first", "sha256:1"),
        make_block("block-a2", "doc-1", "0002", "second", "sha256:2"),
        make_block("block-a3", "doc-1", "0003", "third", "sha256:3"),
    ]);
    let snapshot_b = make_snapshot(vec![
        make_block("block-b3", "doc-1", "0003", "third", "sha256:3"),
        make_block("block-b1", "doc-1", "0001", "first", "sha256:1"),
        make_block("block-b4", "doc-1", "0004", "fourth", "sha256:4"),
    ]);
    let result = compare_documents(&snapshot_a, &snapshot_b, "doc-1", "doc-1");
    let kinds: Vec<BlockDiffKind> = result.blocks.iter().map(|e| e.kind).collect();
    assert_eq!(
        kinds,
        vec![
            BlockDiffKind::Unchanged,
            BlockDiffKind::Removed,
            BlockDiffKind::Unchanged,
            BlockDiffKind::Added,
        ]
    );
    assert_eq!(result.summary.unchanged_count, 2);
    assert_eq!(result.summary.removed_count, 1);
    assert_eq!(result.summary.added_count, 1);
    assert_eq!(result.summary.modified_count, 0);
}

#[test]
fn t06_myers_text_diff_covers_empty_insert_delete_and_mixed() {
    let snapshot_empty = make_snapshot(vec![]);
    let snapshot_text = make_snapshot(vec![make_block(
        "block-t",
        "doc-1",
        "0001",
        "abcdef",
        "sha256:t",
    )]);
    let empty_to_text = compare_documents(&snapshot_empty, &snapshot_text, "doc-1", "doc-1");
    let entry = &empty_to_text.blocks[0];
    assert_eq!(entry.kind, BlockDiffKind::Added);
    let text_to_empty = compare_documents(&snapshot_text, &snapshot_empty, "doc-1", "doc-1");
    let entry = &text_to_empty.blocks[0];
    assert_eq!(entry.kind, BlockDiffKind::Removed);

    let snapshot_a = make_snapshot(vec![make_block(
        "block-a",
        "doc-1",
        "0001",
        "abc",
        "sha256:1",
    )]);
    let snapshot_b = make_snapshot(vec![make_block(
        "block-b",
        "doc-1",
        "0001",
        "axc",
        "sha256:2",
    )]);
    let result = compare_documents(&snapshot_a, &snapshot_b, "doc-1", "doc-1");
    let entry = &result.blocks[0];
    assert_eq!(entry.kind, BlockDiffKind::Modified);
    let ops = entry.text_diff.as_ref().expect("text diff set");
    let kinds: Vec<&str> = ops
        .iter()
        .map(|op| match op {
            TextDiffOp::Equal(_) => "equal",
            TextDiffOp::Insert(_) => "insert",
            TextDiffOp::Delete(_) => "delete",
        })
        .collect();
    assert!(kinds.contains(&"equal"));
    assert!(kinds.contains(&"delete"));
    assert!(kinds.contains(&"insert"));
    assert!(kinds.len() <= 5);
}

#[test]
fn t07_diff_summary_counts_match_block_states() {
    let snapshot_a = make_snapshot(vec![
        make_block("block-a1", "doc-1", "0001", "keep", "sha256:1"),
        make_block("block-a2", "doc-1", "0002", "remove me", "sha256:2"),
        make_block("block-a3", "doc-1", "0003", "edit me", "sha256:3"),
    ]);
    let snapshot_b = make_snapshot(vec![
        make_block("block-b1", "doc-1", "0001", "keep", "sha256:1"),
        make_block("block-b3", "doc-1", "0003", "edited", "sha256:4"),
        make_block("block-b4", "doc-1", "0004", "new", "sha256:5"),
    ]);
    let result = compare_documents(&snapshot_a, &snapshot_b, "doc-1", "doc-1");
    let added = result
        .blocks
        .iter()
        .filter(|e| e.kind == BlockDiffKind::Added)
        .count() as i64;
    let removed = result
        .blocks
        .iter()
        .filter(|e| e.kind == BlockDiffKind::Removed)
        .count() as i64;
    let modified = result
        .blocks
        .iter()
        .filter(|e| e.kind == BlockDiffKind::Modified)
        .count() as i64;
    let unchanged = result
        .blocks
        .iter()
        .filter(|e| e.kind == BlockDiffKind::Unchanged)
        .count() as i64;
    assert_eq!(result.summary.added_count, added);
    assert_eq!(result.summary.removed_count, removed);
    assert_eq!(result.summary.modified_count, modified);
    assert_eq!(result.summary.unchanged_count, unchanged);
    assert_eq!(
        result.blocks.len() as i64,
        added + removed + modified + unchanged
    );
}

#[test]
fn t08_document_diff_result_serde_round_trip() {
    let result = DocumentDiffResult {
        document_id_a: "doc-1".into(),
        document_id_b: "doc-2".into(),
        blocks: vec![
            BlockDiffEntry {
                block_id_a: Some("block-1".into()),
                block_id_b: Some("block-2".into()),
                kind: BlockDiffKind::Modified,
                text_diff: Some(vec![
                    TextDiffOp::Equal("hello".into()),
                    TextDiffOp::Delete(" world".into()),
                    TextDiffOp::Insert(" there".into()),
                ]),
            },
            BlockDiffEntry {
                block_id_a: None,
                block_id_b: Some("block-3".into()),
                kind: BlockDiffKind::Added,
                text_diff: None,
            },
        ],
        summary: DiffSummary {
            added_count: 1,
            removed_count: 0,
            modified_count: 1,
            unchanged_count: 0,
        },
    };
    let json = serde_json::to_string(&result).expect("serialize");
    let parsed: DocumentDiffResult = serde_json::from_str(&json).expect("deserialize");
    assert_eq!(result, parsed);
    let value: serde_json::Value = serde_json::from_str(&json).expect("parse json");
    assert_eq!(value["documentIdA"], serde_json::json!("doc-1"));
    assert_eq!(value["documentIdB"], serde_json::json!("doc-2"));
    assert_eq!(value["blocks"][0]["kind"], serde_json::json!("modified"));
    assert_eq!(value["blocks"][1]["kind"], serde_json::json!("added"));
    assert_eq!(value["blocks"][0]["textDiff"][0]["equal"], serde_json::json!("hello"));
    assert_eq!(value["blocks"][0]["textDiff"][1]["delete"], serde_json::json!(" world"));
    assert_eq!(value["blocks"][0]["textDiff"][2]["insert"], serde_json::json!(" there"));
    assert_eq!(value["summary"]["addedCount"], serde_json::json!(1));
    assert_eq!(value["summary"]["modifiedCount"], serde_json::json!(1));
}

#[test]
fn t09_existing_optimizer_store_tests_remain_unbroken() {
    let snapshot_a = make_snapshot(vec![make_block(
        "block-a",
        "doc-1",
        "0001",
        "hello",
        hash_of("hello").as_str(),
    )]);
    let encoded = optimizer_store::encode_snapshot(&snapshot_a).expect("encode");
    let decoded =
        optimizer_store::decode_snapshot(&encoded.payload, &encoded.checksum).expect("decode");
    assert_eq!(snapshot_a, decoded);
    assert_eq!(SNAPSHOT_CODEC, "optimizer-json+zstd");
    assert_eq!(SNAPSHOT_CODEC_VERSION, 1);
}

// ============================================================
// Stage 2 — lcs_hash_index_correctness 测试模块 (T01-T05)
//
// 验证 lcs_by_order_key 在 hash 索引加速路径下对 5 类场景的正确性。
// 小输入 (N*M <= 4096) 始终走 hash 路径，不触发 DP fallback。
// ============================================================
mod lcs_hash_index_correctness {
    use super::*;

    /// 构造单个 SnapshotBlock（document_id 固定 "doc-1"，plain_text 固定 "text"）。
    fn blk(id: &str, order_key: &str, content_hash: &str) -> SnapshotBlock {
        make_block(id, "doc-1", order_key, "text", content_hash)
    }

    /// 校验 LCS 结果是合法的公共子序列：a/b 索引严格递增且 order_key 匹配。
    fn assert_valid_lcs(
        a: &[&SnapshotBlock],
        b: &[&SnapshotBlock],
        lcs: &[(usize, usize)],
    ) {
        let mut prev_i: i64 = -1;
        let mut prev_j: i64 = -1;
        for &(i, j) in lcs {
            assert!(
                (i as i64) > prev_i,
                "a-index must be strictly increasing; got {i} after {prev_i}"
            );
            assert!(
                (j as i64) > prev_j,
                "b-index must be strictly increasing; got {j} after {prev_j}"
            );
            assert!(i < a.len(), "a-index {i} out of bounds (len {})", a.len());
            assert!(j < b.len(), "b-index {j} out of bounds (len {})", b.len());
            assert_eq!(
                a[i].order_key, b[j].order_key,
                "order_key must match at pair ({i}, {j})"
            );
            prev_i = i as i64;
            prev_j = j as i64;
        }
    }

    // T01: 空 blocks 返回空 LCS
    #[test]
    fn t01_empty_blocks_returns_empty_lcs() {
        let a: Vec<&SnapshotBlock> = vec![];
        let b: Vec<&SnapshotBlock> = vec![];
        let lcs = lcs_by_order_key(&a, &b);
        assert!(lcs.is_empty(), "empty inputs must produce empty LCS");

        // 单侧为空也必须返回空。
        let ba = blk("a0", "0001", "h0");
        let a_only: Vec<&SnapshotBlock> = vec![&ba];
        let empty: Vec<&SnapshotBlock> = vec![];
        assert!(lcs_by_order_key(&a_only, &empty).is_empty());
        assert!(lcs_by_order_key(&empty, &a_only).is_empty());
    }

    // T02: 单 block 匹配
    #[test]
    fn t02_single_block_match() {
        let ba = blk("a1", "0001", "h1");
        let bb = blk("b1", "0001", "h1");
        let a = vec![&ba];
        let b = vec![&bb];
        let lcs = lcs_by_order_key(&a, &b);
        assert_eq!(lcs, vec![(0, 0)]);
        assert_valid_lcs(&a, &b, &lcs);

        // 单 block 不匹配 → 空 LCS。
        let ba2 = blk("a2", "0001", "h1");
        let bb2 = blk("b2", "9999", "h1");
        let a2 = vec![&ba2];
        let b2 = vec![&bb2];
        assert!(lcs_by_order_key(&a2, &b2).is_empty());
    }

    // T03: 重复 order_key 正确处理
    #[test]
    fn t03_duplicate_order_key_handled_correctly() {
        // a 含两个 "X"，b 含一个 "X"；LCS 只能匹配一个 X + 一个 Y。
        let ba0 = blk("a0", "X", "h0");
        let ba1 = blk("a1", "X", "h1");
        let ba2 = blk("a2", "Y", "h2");
        let bb0 = blk("b0", "X", "h0");
        let bb1 = blk("b1", "Y", "h2");
        let a = vec![&ba0, &ba1, &ba2];
        let b = vec![&bb0, &bb1];
        let lcs = lcs_by_order_key(&a, &b);
        // 期望 LCS 长度 = 2（一个 X + 一个 Y）。
        assert_eq!(lcs.len(), 2, "LCS length must be 2 for [X,X,Y] vs [X,Y]");
        assert_valid_lcs(&a, &b, &lcs);

        // 双侧均有重复：a=[X,X,Y,Y], b=[X,Y,Y]。
        let ca0 = blk("ca0", "X", "h0");
        let ca1 = blk("ca1", "X", "h1");
        let ca2 = blk("ca2", "Y", "h2");
        let ca3 = blk("ca3", "Y", "h3");
        let cb0 = blk("cb0", "X", "h0");
        let cb1 = blk("cb1", "Y", "h2");
        let cb2 = blk("cb2", "Y", "h3");
        let a2 = vec![&ca0, &ca1, &ca2, &ca3];
        let b2 = vec![&cb0, &cb1, &cb2];
        let lcs2 = lcs_by_order_key(&a2, &b2);
        // 期望 LCS 长度 = 3（一个 X + 两个 Y）。
        assert_eq!(
            lcs2.len(),
            3,
            "LCS length must be 3 for [X,X,Y,Y] vs [X,Y,Y]"
        );
        assert_valid_lcs(&a2, &b2, &lcs2);
    }

    // T04: 全 unchanged 场景
    #[test]
    fn t04_all_unchanged_scenario() {
        let ba0 = blk("a0", "0001", "h1");
        let ba1 = blk("a1", "0002", "h2");
        let ba2 = blk("a2", "0003", "h3");
        let bb0 = blk("b0", "0001", "h1");
        let bb1 = blk("b1", "0002", "h2");
        let bb2 = blk("b2", "0003", "h3");
        let a = vec![&ba0, &ba1, &ba2];
        let b = vec![&bb0, &bb1, &bb2];
        let lcs = lcs_by_order_key(&a, &b);
        // 全匹配 → LCS 长度 = 3，按序对齐。
        assert_eq!(lcs, vec![(0, 0), (1, 1), (2, 2)]);
        assert_valid_lcs(&a, &b, &lcs);
    }

    // T05: 全 modified 场景
    #[test]
    fn t05_all_modified_scenario() {
        // order_key 相同但 content_hash 不同 — LCS 仍按 order_key 对齐，
        // 长度 = 2（LCS 不关心 content_hash）。
        let ba0 = blk("a0", "0001", "h1");
        let ba1 = blk("a1", "0002", "h2");
        let bb0 = blk("b0", "0001", "hX");
        let bb1 = blk("b1", "0002", "hY");
        let a = vec![&ba0, &ba1];
        let b = vec![&bb0, &bb1];
        let lcs = lcs_by_order_key(&a, &b);
        assert_eq!(lcs, vec![(0, 0), (1, 1)]);
        assert_valid_lcs(&a, &b, &lcs);
    }
}

// ============================================================
// Stage 2 — T07: compare_documents 早停启发式
// content_hash 相同 → 直接 Unchanged，跳过 myers_diff（text_diff 为 None）
// ============================================================
#[test]
fn t07_compare_documents_early_stop_content_hash_skips_text_diff() {
    // 构造两个 block：order_key 相同、content_hash 相同，但 plain_text 不同。
    // 若 myers_diff 被调用，text_diff 会是非空 Some(...)；早停应使其为 None。
    let block_a = make_block("block-a", "doc-1", "0001", "hello world", "sha256:same");
    let block_b = make_block("block-b", "doc-1", "0001", "hello there", "sha256:same");
    let snapshot_a = make_snapshot(vec![block_a]);
    let snapshot_b = make_snapshot(vec![block_b]);
    let result = compare_documents(&snapshot_a, &snapshot_b, "doc-1", "doc-1");

    assert_eq!(result.blocks.len(), 1, "exactly one matched block expected");
    let entry = &result.blocks[0];
    assert_eq!(
        entry.kind,
        BlockDiffKind::Unchanged,
        "identical content_hash must yield Unchanged (early-stop)"
    );
    assert!(
        entry.text_diff.is_none(),
        "text_diff must be None when content_hash matches (myers_diff skipped)"
    );
    assert_eq!(result.summary.unchanged_count, 1);
    assert_eq!(result.summary.modified_count, 0);

    // 对照组：content_hash 不同 → Modified + 非空 text_diff。
    let block_c = make_block("block-c", "doc-1", "0001", "hello world", "sha256:diff-a");
    let block_d = make_block("block-d", "doc-1", "0001", "hello there", "sha256:diff-b");
    let snapshot_c = make_snapshot(vec![block_c]);
    let snapshot_d = make_snapshot(vec![block_d]);
    let result2 = compare_documents(&snapshot_c, &snapshot_d, "doc-1", "doc-1");
    let entry2 = &result2.blocks[0];
    assert_eq!(entry2.kind, BlockDiffKind::Modified);
    assert!(
        entry2.text_diff.is_some(),
        "text_diff must be Some when content_hash differs"
    );
    assert!(
        !entry2
            .text_diff
            .as_ref()
            .expect("text_diff present")
            .is_empty(),
        "myers_diff on differing text must produce non-empty ops"
    );
}

// ============================================================
// Stage 2 — T08: compare_documents 变更区域裁剪
// 纯增 / 纯删场景下 Added / Removed 的 text_diff 必须为 None（不调用 myers_diff）
// ============================================================
#[test]
fn t08_compare_documents_change_region_clipping_skips_myers_for_add_remove() {
    // 纯增：snapshot_a 无 blocks，snapshot_b 有 3 个 blocks。
    let snapshot_a_empty = make_snapshot(vec![]);
    let snapshot_b_full = make_snapshot(vec![
        make_block("block-b1", "doc-1", "0001", "first", "sha256:b1"),
        make_block("block-b2", "doc-1", "0002", "second", "sha256:b2"),
        make_block("block-b3", "doc-1", "0003", "third", "sha256:b3"),
    ]);
    let added = compare_documents(&snapshot_a_empty, &snapshot_b_full, "doc-1", "doc-1");
    assert_eq!(added.summary.added_count, 3);
    assert_eq!(added.summary.removed_count, 0);
    assert_eq!(added.summary.modified_count, 0);
    assert_eq!(added.summary.unchanged_count, 0);
    for entry in &added.blocks {
        assert_eq!(entry.kind, BlockDiffKind::Added);
        assert!(
            entry.text_diff.is_none(),
            "Added blocks must not invoke myers_diff (text_diff must be None)"
        );
        assert!(entry.block_id_a.is_none());
        assert!(entry.block_id_b.is_some());
    }

    // 纯删：snapshot_a 有 3 个 blocks，snapshot_b 无 blocks。
    let snapshot_a_full = make_snapshot(vec![
        make_block("block-a1", "doc-1", "0001", "first", "sha256:a1"),
        make_block("block-a2", "doc-1", "0002", "second", "sha256:a2"),
        make_block("block-a3", "doc-1", "0003", "third", "sha256:a3"),
    ]);
    let snapshot_b_empty = make_snapshot(vec![]);
    let removed = compare_documents(&snapshot_a_full, &snapshot_b_empty, "doc-1", "doc-1");
    assert_eq!(removed.summary.removed_count, 3);
    assert_eq!(removed.summary.added_count, 0);
    assert_eq!(removed.summary.modified_count, 0);
    assert_eq!(removed.summary.unchanged_count, 0);
    for entry in &removed.blocks {
        assert_eq!(entry.kind, BlockDiffKind::Removed);
        assert!(
            entry.text_diff.is_none(),
            "Removed blocks must not invoke myers_diff (text_diff must be None)"
        );
        assert!(entry.block_id_a.is_some());
        assert!(entry.block_id_b.is_none());
    }

    // 混合场景：1 unchanged + 1 added + 1 removed，只有 matched pair 可能调用 myers_diff，
    // 此处 content_hash 相同 → Unchanged，故全部分支均无 text_diff。
    let snapshot_mix_a = make_snapshot(vec![
        make_block("block-ma1", "doc-1", "0001", "keep", "sha256:keep"),
        make_block("block-ma2", "doc-1", "0002", "drop", "sha256:drop"),
    ]);
    let snapshot_mix_b = make_snapshot(vec![
        make_block("block-mb1", "doc-1", "0001", "keep", "sha256:keep"),
        make_block("block-mb3", "doc-1", "0003", "new", "sha256:new"),
    ]);
    let mixed = compare_documents(&snapshot_mix_a, &snapshot_mix_b, "doc-1", "doc-1");
    for entry in &mixed.blocks {
        assert!(
            entry.text_diff.is_none(),
            "no Modified blocks in this mix → no text_diff allowed; got {:?} on {:?}",
            entry.text_diff,
            entry.kind
        );
    }
}
