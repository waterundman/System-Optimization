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
use std::time::{Duration, Instant};

use optimizer_store::snapshot::{
    ProjectSnapshotV1, SnapshotBlock, SnapshotBranch, SnapshotCommit, SnapshotDocument,
    SnapshotProject, SNAPSHOT_SCHEMA_VERSION, decode_snapshot, encode_snapshot,
};

// ============================================================
// 性能门禁阈值（perf regression gate）
//
// 采数日期：2026-09-09，本机实测（Windows 11，cargo test 默认 debug profile）：
// - 10k blocks encode: 197.9ms / decode: 117.3ms
// - 100k blocks encode: 1.710s / decode: 829.8ms
// - zstd 各 level（1/3/6/9）encode 最慢 208.5ms（level 9）、decode 最慢 12.2ms
//
// 阈值 = max(实测 × 3, 1s)，3 倍余量用于吸收 GitHub runner 与本机性能差。
// 调整阈值前请先 `cargo test -p optimizer-store -- --ignored --nocapture` 重新采数。
// ============================================================
/// 实测 197.9ms × 3 ≈ 594ms < 1s，取下限 1s
const ENCODE_10K_THRESHOLD: Duration = Duration::from_secs(1);
/// 实测 117.3ms × 3 ≈ 352ms < 1s，取下限 1s
const DECODE_10K_THRESHOLD: Duration = Duration::from_secs(1);
/// 实测 1.710s × 3 ≈ 5.13s，向上取整 5.2s
const ENCODE_100K_THRESHOLD: Duration = Duration::from_millis(5_200);
/// 实测 829.8ms × 3 ≈ 2.49s，向上取整 2.5s
const DECODE_100K_THRESHOLD: Duration = Duration::from_millis(2_500);
/// 最慢档（level 9）encode 实测 208.5ms × 3 < 1s，取下限 1s
const ZSTD_LEVEL_ENCODE_THRESHOLD: Duration = Duration::from_secs(1);
/// 最慢档 decode 实测 12.2ms × 3 < 1s，取下限 1s
const ZSTD_LEVEL_DECODE_THRESHOLD: Duration = Duration::from_secs(1);

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
// T06 — 10k blocks encode/decode 性能门禁（阈值见文件顶部常量）
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
        encode_duration <= ENCODE_10K_THRESHOLD,
        "perf regression: actual={:?} threshold={:?}",
        encode_duration,
        ENCODE_10K_THRESHOLD
    );

    let start = Instant::now();
    let decoded = decode_snapshot(&encoded.payload, &encoded.checksum).expect("decode failed");
    let decode_duration = start.elapsed();
    println!("10k blocks decode: {:?}", decode_duration);
    assert!(
        decode_duration <= DECODE_10K_THRESHOLD,
        "perf regression: actual={:?} threshold={:?}",
        decode_duration,
        DECODE_10K_THRESHOLD
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
        assert!(
            duration <= ZSTD_LEVEL_ENCODE_THRESHOLD,
            "perf regression: level={level} actual={:?} threshold={:?}",
            duration,
            ZSTD_LEVEL_ENCODE_THRESHOLD
        );
    }
}

// ============================================================
// T01/T02 — 100k blocks encode/decode 性能门禁（阈值见文件顶部常量）
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
        encode_duration <= ENCODE_100K_THRESHOLD,
        "perf regression: actual={:?} threshold={:?}",
        encode_duration,
        ENCODE_100K_THRESHOLD
    );

    let start = Instant::now();
    let decoded = decode_snapshot(&encoded.payload, &encoded.checksum).expect("decode failed");
    let decode_duration = start.elapsed();
    println!("100k blocks decode: {:?}", decode_duration);
    assert!(
        decode_duration <= DECODE_100K_THRESHOLD,
        "perf regression: actual={:?} threshold={:?}",
        decode_duration,
        DECODE_100K_THRESHOLD
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
        assert!(
            encode_duration <= ZSTD_LEVEL_ENCODE_THRESHOLD,
            "perf regression: level={level} encode actual={:?} threshold={:?}",
            encode_duration,
            ZSTD_LEVEL_ENCODE_THRESHOLD
        );
        assert!(
            decode_duration <= ZSTD_LEVEL_DECODE_THRESHOLD,
            "perf regression: level={level} decode actual={:?} threshold={:?}",
            decode_duration,
            ZSTD_LEVEL_DECODE_THRESHOLD
        );
    }

    println!("note: default encode_snapshot level remains 3 (no switch)");
}
