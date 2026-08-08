//! 10k blocks 快照编解码性能基线测试
//!
//! 标记 `#[ignore]`，默认不跑，通过 `cargo test -- --ignored` 显式触发。
//!
//! 测量维度：
//! - 10k blocks 编码耗时 + 压缩后字节数
//! - 10k blocks 解码耗时
//! - zstd level 1/3/6/9 编码耗时与压缩率对比（直接调用 zstd crate，绕过 encode_snapshot 的硬编码 level=3）

#![cfg(test)]

use std::io::Cursor;
use std::time::Instant;

use optimizer_store::snapshot::{
    ProjectSnapshotV1, SnapshotBlock, SnapshotBranch, SnapshotCommit, SnapshotDocument,
    SnapshotProject, SNAPSHOT_SCHEMA_VERSION, decode_snapshot, encode_snapshot,
};

/// 生成包含 10k SnapshotBlock 的 ProjectSnapshotV1。
fn generate_10k_blocks_snapshot() -> ProjectSnapshotV1 {
    let blocks: Vec<SnapshotBlock> = (0..10_000)
        .map(|i| {
            let plain_text = format!("Block {i} content for performance testing");
            let content_json = format!(
                r#"{{"kind":"paragraph","text":"Block {i} content for performance testing"}}"#
            );
            // 简易确定性 hash（性能测试不需要密码学强度）
            let content_hash = format!("sha256:block-{i:06}");
            SnapshotBlock {
                id: format!("block-{i:06}"),
                document_id: "doc-1".to_string(),
                kind: "paragraph".to_string(),
                order_key: format!("{i:06}"),
                content_json,
                plain_text,
                content_hash,
                revision: 1,
                locked: false,
            }
        })
        .collect();

    ProjectSnapshotV1 {
        schema_version: SNAPSHOT_SCHEMA_VERSION,
        project: SnapshotProject {
            id: "proj-1".into(),
            title: "Performance Baseline Project".into(),
            language: "zh-CN".into(),
        },
        commit: SnapshotCommit {
            id: "commit-1".into(),
            root_hash: "sha256:root-baseline".into(),
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

// ============================================================
// T06 — 10k blocks encode < 2s, decode < 1s（基线）
// ============================================================
#[test]
#[ignore]
fn snapshot_encode_10k_blocks_baseline() {
    let snapshot = generate_10k_blocks_snapshot();

    let start = Instant::now();
    let encoded = encode_snapshot(&snapshot).expect("encode failed");
    let encode_duration = start.elapsed();
    println!(
        "10k blocks encode: {:?} ({} bytes compressed, checksum={})",
        encode_duration,
        encoded.payload.len(),
        encoded.checksum
    );
    assert!(
        encode_duration.as_secs() < 2,
        "encode took {:?}, expected < 2s",
        encode_duration
    );

    let start = Instant::now();
    let decoded = decode_snapshot(&encoded.payload, &encoded.checksum).expect("decode failed");
    let decode_duration = start.elapsed();
    println!("10k blocks decode: {:?}", decode_duration);
    assert!(
        decode_duration.as_secs() < 1,
        "decode took {:?}, expected < 1s",
        decode_duration
    );

    assert_eq!(snapshot, decoded, "roundtrip should preserve snapshot");
}

// ============================================================
// zstd level 1/3/6/9 编码耗时与压缩率对比
// ============================================================
#[test]
#[ignore]
fn snapshot_zstd_level_comparison() {
    let snapshot = generate_10k_blocks_snapshot();
    let json = serde_json::to_vec(&snapshot).expect("serialize snapshot failed");

    println!("raw JSON size: {} bytes", json.len());
    println!("level | compressed bytes | encode duration");
    println!("-------+-----------------+------------------");

    for level in [1, 3, 6, 9] {
        let start = Instant::now();
        let compressed = zstd::stream::encode_all(Cursor::new(&json), level)
            .expect("zstd encode_all failed");
        let duration = start.elapsed();
        println!(
            "  {:>2}  | {:>15} | {:?}",
            level,
            compressed.len(),
            duration
        );
    }
}

// ============================================================
// T01/T02 — 100k blocks encode < 3s, decode < 1.5s（基线）
// ============================================================
/// 生成包含 100k SnapshotBlock 的 ProjectSnapshotV1。
fn generate_100k_blocks_snapshot() -> ProjectSnapshotV1 {
    let blocks: Vec<SnapshotBlock> = (0..100_000)
        .map(|i| {
            let plain_text = format!("Block {i} content for performance testing at scale");
            let content_json = format!(
                r#"{{"kind":"paragraph","text":"Block {i} content for performance testing at scale"}}"#
            );
            let content_hash = format!("sha256:block-{i:06}");
            SnapshotBlock {
                id: format!("block-{i:06}"),
                document_id: "doc-1".to_string(),
                kind: "paragraph".to_string(),
                order_key: format!("{i:06}"),
                content_json,
                plain_text,
                content_hash,
                revision: 1,
                locked: false,
            }
        })
        .collect();

    ProjectSnapshotV1 {
        schema_version: SNAPSHOT_SCHEMA_VERSION,
        project: SnapshotProject {
            id: "proj-1".into(),
            title: "Performance Baseline Project 100k".into(),
            language: "zh-CN".into(),
        },
        commit: SnapshotCommit {
            id: "commit-1".into(),
            root_hash: "sha256:root-baseline-100k".into(),
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

#[test]
#[ignore]
fn snapshot_encode_100k_blocks_baseline() {
    let snapshot = generate_100k_blocks_snapshot();

    let start = Instant::now();
    let encoded = encode_snapshot(&snapshot).expect("encode failed");
    let encode_duration = start.elapsed();
    println!(
        "100k blocks encode: {:?} ({} bytes compressed, checksum={})",
        encode_duration,
        encoded.payload.len(),
        encoded.checksum
    );
    assert!(
        encode_duration.as_secs() < 3,
        "encode took {:?}, expected < 3s",
        encode_duration
    );

    let start = Instant::now();
    let decoded = decode_snapshot(&encoded.payload, &encoded.checksum).expect("decode failed");
    let decode_duration = start.elapsed();
    println!("100k blocks decode: {:?}", decode_duration);
    assert!(
        decode_duration.as_millis() < 1500,
        "decode took {:?}, expected < 1.5s",
        decode_duration
    );

    assert_eq!(
        snapshot.blocks.len(),
        decoded.blocks.len(),
        "block count should be preserved"
    );
    assert_eq!(snapshot, decoded, "roundtrip should preserve snapshot");
}

// ============================================================
// T03 — zstd level 1/3/6/9 encode + ratio + decode 对比（不切换默认）
// ============================================================
#[test]
#[ignore]
fn zstd_level_comparison_baseline() {
    let snapshot = generate_10k_blocks_snapshot();
    let json = serde_json::to_vec(&snapshot).expect("serialize snapshot failed");

    println!("raw JSON size: {} bytes", json.len());
    println!(
        "level | compressed bytes | ratio   | encode duration | decode duration"
    );
    println!("-------+-----------------+---------+-----------------+----------------");

    for level in [1, 3, 6, 9] {
        let start = Instant::now();
        let compressed = zstd::stream::encode_all(Cursor::new(&json), level)
            .expect("zstd encode_all failed");
        let encode_duration = start.elapsed();

        let ratio = json.len() as f64 / compressed.len() as f64;

        let start = Instant::now();
        let mut decoder = zstd::stream::read::Decoder::new(Cursor::new(&compressed))
            .expect("zstd decoder failed");
        let mut decompressed = Vec::new();
        std::io::Read::read_to_end(&mut decoder, &mut decompressed)
            .expect("zstd decode failed");
        let decode_duration = start.elapsed();

        assert_eq!(decompressed, json, "zstd roundtrip failed at level {level}");

        println!(
            "  {:>2}  | {:>15} | {:>5.2}x  | {:>15?} | {:?}",
            level,
            compressed.len(),
            ratio,
            encode_duration,
            decode_duration
        );
    }

    println!("note: default encode_snapshot level remains 3 (no switch)");
}
