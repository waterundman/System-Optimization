//! Stage 2 — payload_hash 变形追踪测试 (v0.7.0)
//!
//! 验证 `get_payload_hash_variations` store 方法：
//! - `total_proposals`: 至少有一个 `patch_review_event` 的 proposal 总数
//! - `proposals_with_variation`: 同一 proposal_id 下出现 2+ 不同 payload_hash 的 proposal 数
//! - `variations`: 总变形次数（所有 proposal 的不同 payload_hash 数之和减去有 hash 的 proposal 数）
//! - `variation_rate = proposals_with_variation / max(total_proposals, 1)`
//!
//! 核心设计约束：
//! - 复用 `patch_review_event.payload_json` JSON 字段存储 `{"payload_hash":"sha256:..."}`
//! - 不新增表，不新增列（schema 兼容性优先）
//! - `payload_json` 缺失 `payload_hash` 时 graceful 处理（不崩溃，该 proposal 不计入变形）
//! - 冷启动 `variation_rate = 0.0`（分母 `max(total_proposals, 1)` 避免除零）

#![cfg(test)]

use optimizer_store::{
    AppendReviewEvent, OptimizerStore, PersistOperationBundle, ProjectSeed, ReviewDecision,
    ReviewEventKind, ReviewSessionStatus, SeedBlock, SeedDocument,
};
use rusqlite::params;

// ============================================================
// 测试基础设施
// ============================================================

/// 构造一个已初始化 project + commit_node 的内存 store。
fn seeded_store() -> OptimizerStore {
    let mut store = OptimizerStore::open_in_memory().expect("open_in_memory failed");
    store
        .initialize_project(&ProjectSeed {
            project_id: "proj-1".into(),
            title: "Payload Hash Variations Test Project".into(),
            language: "zh-CN".into(),
            initial_commit_id: "commit-1".into(),
            initial_root_hash: "sha256:root-initial".into(),
            main_branch_id: "branch-main".into(),
            documents: vec![SeedDocument {
                id: "document-1".into(),
                parent_id: None,
                kind: "chapter".into(),
                title: "Chapter".into(),
                order_key: "a0".into(),
            }],
            blocks: vec![SeedBlock {
                id: "block-1".into(),
                document_id: "document-1".into(),
                kind: "paragraph".into(),
                order_key: "a0".into(),
                content_json: r#"{"type":"paragraph"}"#.into(),
                plain_text: "hello".into(),
                content_hash: "sha256:block-1".into(),
                locked: false,
            }],
            created_at: "2026-07-14T00:00:00.000Z".into(),
        })
        .expect("initialize_project failed");
    store
}

/// 构造一个最小的 PersistOperationBundle，对应一个 patch_proposal artifact。
fn proposal_bundle(run_id: &str, packet_id: &str, proposal_id: &str) -> PersistOperationBundle {
    use optimizer_store::{
        ModelUsageRecord, NewContextPacket, NewOperationArtifact, NewOperationAttempt,
        NewOperationLifecycleEvent, NewOperationRun, OperationArtifactKind, OperationState,
    };
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
            project_id: "proj-1".into(),
            base_commit_id: "commit-1".into(),
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
            project_id: "proj-1".into(),
            base_commit_id: "commit-1".into(),
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
        attempts: vec![NewOperationAttempt {
            sequence: 1,
            started_at: "2026-07-15T00:00:20.000Z".into(),
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
        }],
        artifact: Some(NewOperationArtifact {
            id: proposal_id.into(),
            kind: OperationArtifactKind::PatchProposal,
            binding_hash: format!("sha256:{proposal_id}"),
            payload_json: format!(r#"{{"id":"{proposal_id}","schemaVersion":2}}"#),
            created_at: "2026-07-15T00:01:00.000Z".into(),
        }),
    }
}

/// 构造 payload_hash JSON 字符串。
fn payload_json_with_hash(hash: &str) -> Option<String> {
    Some(format!(r#"{{"payload_hash":"{hash}"}}"#))
}

/// 通过 store API 添加一个带 payload_hash 的 decision 事件（review → ready）。
#[allow(clippy::too_many_arguments)]
fn append_decision_with_hash(
    store: &mut OptimizerStore,
    id: &str,
    proposal_id: &str,
    expected_revision: i64,
    hunk_id: &str,
    decision: ReviewDecision,
    next_status: ReviewSessionStatus,
    occurred_at: &str,
    payload_hash: Option<&str>,
) {
    store
        .append_review_event(&AppendReviewEvent {
            id: id.into(),
            proposal_id: proposal_id.into(),
            expected_revision,
            expected_status: ReviewSessionStatus::Review,
            kind: ReviewEventKind::Decision,
            next_status,
            hunk_id: Some(hunk_id.into()),
            decision: Some(decision),
            payload_json: payload_hash.and_then(payload_json_with_hash),
            occurred_at: occurred_at.into(),
        })
        .expect("append_decision_with_hash failed");
}

/// 通过 store API 添加一个带 payload_hash 的 apply 事件（ready → applied）。
fn append_apply_with_hash(
    store: &mut OptimizerStore,
    id: &str,
    proposal_id: &str,
    expected_revision: i64,
    expected_status: ReviewSessionStatus,
    occurred_at: &str,
    payload_hash: Option<&str>,
) {
    store
        .append_review_event(&AppendReviewEvent {
            id: id.into(),
            proposal_id: proposal_id.into(),
            expected_revision,
            expected_status,
            kind: ReviewEventKind::Apply,
            next_status: ReviewSessionStatus::Applied,
            hunk_id: None,
            decision: None,
            payload_json: payload_hash.and_then(payload_json_with_hash),
            occurred_at: occurred_at.into(),
        })
        .expect("append_apply_with_hash failed");
}

// ============================================================
// T01 — 空 project 返回全零（冷启动 variation_rate=0.0）
// ============================================================
#[test]
fn t01_empty_project_returns_zero_variations() {
    let store = seeded_store();
    let metrics = store
        .get_payload_hash_variations("proj-1")
        .expect("get_payload_hash_variations on empty project failed");
    assert_eq!(metrics.total_proposals, 0);
    assert_eq!(metrics.proposals_with_variation, 0);
    assert_eq!(metrics.variations, 0);
    assert_eq!(metrics.variation_rate, 0.0);
}

// ============================================================
// T02 — 无变形 proposal（所有事件 payload_hash 相同）不计入 variations
// ============================================================
#[test]
fn t02_no_variation_when_all_hashes_identical() {
    let mut store = seeded_store();
    store
        .persist_operation_bundle(&proposal_bundle("run-a", "ctx-a", "proposal-a"))
        .expect("persist bundle failed");

    // review → ready (decision) with payload_hash = sha256:same
    append_decision_with_hash(
        &mut store,
        "event-a-1",
        "proposal-a",
        0,
        "hunk-a",
        ReviewDecision::Accepted,
        ReviewSessionStatus::Ready,
        "2026-07-15T00:02:00.000Z",
        Some("sha256:same"),
    );
    // ready → applied (apply) with payload_hash = sha256:same (相同)
    append_apply_with_hash(
        &mut store,
        "event-a-2",
        "proposal-a",
        1,
        ReviewSessionStatus::Ready,
        "2026-07-15T00:03:00.000Z",
        Some("sha256:same"),
    );

    let metrics = store
        .get_payload_hash_variations("proj-1")
        .expect("get_payload_hash_variations failed");
    assert_eq!(metrics.total_proposals, 1);
    assert_eq!(metrics.proposals_with_variation, 0);
    assert_eq!(metrics.variations, 0);
    // 0 / max(1, 1) = 0.0
    assert_eq!(metrics.variation_rate, 0.0);
}

// ============================================================
// T03 — 有变形 proposal（2+ 不同 payload_hash）计入 variations
// ============================================================
#[test]
fn t03_variation_counted_when_distinct_hashes() {
    let mut store = seeded_store();
    store
        .persist_operation_bundle(&proposal_bundle("run-b", "ctx-b", "proposal-b"))
        .expect("persist bundle failed");

    // review → ready (decision) with payload_hash = sha256:v1
    append_decision_with_hash(
        &mut store,
        "event-b-1",
        "proposal-b",
        0,
        "hunk-b",
        ReviewDecision::Accepted,
        ReviewSessionStatus::Ready,
        "2026-07-15T00:02:00.000Z",
        Some("sha256:v1"),
    );
    // ready → applied (apply) with payload_hash = sha256:v2 (变形)
    append_apply_with_hash(
        &mut store,
        "event-b-2",
        "proposal-b",
        1,
        ReviewSessionStatus::Ready,
        "2026-07-15T00:03:00.000Z",
        Some("sha256:v2"),
    );

    let metrics = store
        .get_payload_hash_variations("proj-1")
        .expect("get_payload_hash_variations failed");
    assert_eq!(metrics.total_proposals, 1);
    assert_eq!(metrics.proposals_with_variation, 1);
    // 2 个不同 hash → variations = 2 - 1 = 1
    assert_eq!(metrics.variations, 1);
    // 1 / max(1, 1) = 1.0
    assert!(
        (metrics.variation_rate - 1.0).abs() < f64::EPSILON,
        "rate should be 1.0, got {}",
        metrics.variation_rate
    );
}

// ============================================================
// T04 — variation_rate 计算正确（多 proposal 比例 + 冷启动 0.0）
// ============================================================
#[test]
fn t04_variation_rate_calculation() {
    let mut store = seeded_store();

    // proposal-X: 无变形（2 事件相同 hash）
    store
        .persist_operation_bundle(&proposal_bundle("run-x", "ctx-x", "proposal-x"))
        .expect("persist X failed");
    append_decision_with_hash(
        &mut store,
        "event-x-1",
        "proposal-x",
        0,
        "hunk-x",
        ReviewDecision::Accepted,
        ReviewSessionStatus::Ready,
        "2026-07-15T00:02:00.000Z",
        Some("sha256:x-hash"),
    );
    append_apply_with_hash(
        &mut store,
        "event-x-2",
        "proposal-x",
        1,
        ReviewSessionStatus::Ready,
        "2026-07-15T00:03:00.000Z",
        Some("sha256:x-hash"),
    );

    // proposal-Y: 有变形（2 事件不同 hash）
    store
        .persist_operation_bundle(&proposal_bundle("run-y", "ctx-y", "proposal-y"))
        .expect("persist Y failed");
    append_decision_with_hash(
        &mut store,
        "event-y-1",
        "proposal-y",
        0,
        "hunk-y",
        ReviewDecision::Accepted,
        ReviewSessionStatus::Ready,
        "2026-07-15T00:02:00.000Z",
        Some("sha256:y-v1"),
    );
    append_apply_with_hash(
        &mut store,
        "event-y-2",
        "proposal-y",
        1,
        ReviewSessionStatus::Ready,
        "2026-07-15T00:03:00.000Z",
        Some("sha256:y-v2"),
    );

    // proposal-Z: 有变形（3 事件 3 个不同 hash → variations = 2）
    store
        .persist_operation_bundle(&proposal_bundle("run-z", "ctx-z", "proposal-z"))
        .expect("persist Z failed");
    append_decision_with_hash(
        &mut store,
        "event-z-1",
        "proposal-z",
        0,
        "hunk-z",
        ReviewDecision::Accepted,
        ReviewSessionStatus::Ready,
        "2026-07-15T00:02:00.000Z",
        Some("sha256:z-v1"),
    );
    append_apply_with_hash(
        &mut store,
        "event-z-2",
        "proposal-z",
        1,
        ReviewSessionStatus::Ready,
        "2026-07-15T00:03:00.000Z",
        Some("sha256:z-v2"),
    );
    // 第三条事件绕过状态机直接插入（applied → applied 不可达，用 SQL）
    let conn = store.connection();
    conn.execute_batch("PRAGMA foreign_keys = OFF")
        .expect("disable FK failed");
    conn.execute(
        "INSERT INTO patch_review_event(
           id, proposal_id, sequence, base_revision, new_revision, kind,
           previous_status, next_status, hunk_id, decision, payload_json, occurred_at
         ) VALUES (?1, ?2, ?3, ?4, ?5, ?6, ?7, ?8, NULL, NULL, ?9, ?10)",
        params![
            "event-z-3",
            "proposal-z",
            3i64,
            2i64,
            3i64,
            "rebase",
            "applied",
            "review",
            r#"{"payload_hash":"sha256:z-v3"}"#,
            "2026-07-15T00:04:00.000Z",
        ],
    )
    .expect("insert z-3 failed");
    conn.execute_batch("PRAGMA foreign_keys = ON")
        .expect("enable FK failed");

    let metrics = store
        .get_payload_hash_variations("proj-1")
        .expect("get_payload_hash_variations failed");
    // 3 个 proposal
    assert_eq!(metrics.total_proposals, 3);
    // proposal-Y(1变形) + proposal-Z(1变形) = 2
    assert_eq!(metrics.proposals_with_variation, 2);
    // variations: Y(2-1=1) + Z(3-1=2) = 3
    assert_eq!(metrics.variations, 3);
    // rate = 2 / max(3, 1) = 0.6666...
    let expected_rate = 2.0 / 3.0;
    assert!(
        (metrics.variation_rate - expected_rate).abs() < f64::EPSILON,
        "rate should be {expected_rate}, got {}",
        metrics.variation_rate
    );

    // 冷启动场景：在另一个空 project 上 rate 必须为 0.0
    let metrics_empty = store
        .get_payload_hash_variations("proj-empty")
        .expect("get_payload_hash_variations on empty project failed");
    assert_eq!(metrics_empty.total_proposals, 0);
    assert_eq!(metrics_empty.proposals_with_variation, 0);
    assert_eq!(metrics_empty.variations, 0);
    assert_eq!(metrics_empty.variation_rate, 0.0);
}

// ============================================================
// T05 — payload_json 缺失 payload_hash 时 graceful 处理（不崩溃）
// ============================================================
#[test]
fn t05_missing_payload_hash_graceful() {
    let mut store = seeded_store();
    store
        .persist_operation_bundle(&proposal_bundle("run-m", "ctx-m", "proposal-m"))
        .expect("persist bundle failed");

    // 事件 1: payload_json = None（缺失）
    append_decision_with_hash(
        &mut store,
        "event-m-1",
        "proposal-m",
        0,
        "hunk-m",
        ReviewDecision::Accepted,
        ReviewSessionStatus::Ready,
        "2026-07-15T00:02:00.000Z",
        None,
    );
    // 事件 2: payload_json 存在但无 payload_hash 字段（graceful）
    store
        .append_review_event(&AppendReviewEvent {
            id: "event-m-2".into(),
            proposal_id: "proposal-m".into(),
            expected_revision: 1,
            expected_status: ReviewSessionStatus::Ready,
            kind: ReviewEventKind::Apply,
            next_status: ReviewSessionStatus::Applied,
            hunk_id: None,
            decision: None,
            payload_json: Some(r#"{"code":"APPLIED","note":"no hash"}"#.into()),
            occurred_at: "2026-07-15T00:03:00.000Z".into(),
        })
        .expect("append_review_event failed");

    let metrics = store
        .get_payload_hash_variations("proj-1")
        .expect("get_payload_hash_variations failed");
    // proposal 有事件，计入 total_proposals
    assert_eq!(metrics.total_proposals, 1);
    // 但无 payload_hash，不计入变形
    assert_eq!(metrics.proposals_with_variation, 0);
    assert_eq!(metrics.variations, 0);
    assert_eq!(metrics.variation_rate, 0.0);

    // 再加一个事件带 payload_hash，验证混合场景不崩溃且正确统计
    let conn = store.connection();
    conn.execute_batch("PRAGMA foreign_keys = OFF")
        .expect("disable FK failed");
    conn.execute(
        "INSERT INTO patch_review_event(
           id, proposal_id, sequence, base_revision, new_revision, kind,
           previous_status, next_status, hunk_id, decision, payload_json, occurred_at
         ) VALUES (?1, ?2, ?3, ?4, ?5, ?6, ?7, ?8, NULL, NULL, ?9, ?10)",
        params![
            "event-m-3",
            "proposal-m",
            3i64,
            2i64,
            3i64,
            "rebase",
            "applied",
            "review",
            r#"{"payload_hash":"sha256:m-v1"}"#,
            "2026-07-15T00:04:00.000Z",
        ],
    )
    .expect("insert m-3 failed");
    conn.execute_batch("PRAGMA foreign_keys = ON")
        .expect("enable FK failed");

    let metrics2 = store
        .get_payload_hash_variations("proj-1")
        .expect("get_payload_hash_variations after m-3 failed");
    // 仍然只有 1 个 proposal
    assert_eq!(metrics2.total_proposals, 1);
    // 只有 1 个 distinct hash（sha256:m-v1），不算变形
    assert_eq!(metrics2.proposals_with_variation, 0);
    assert_eq!(metrics2.variations, 0);
    assert_eq!(metrics2.variation_rate, 0.0);
}

// ============================================================
// T06 — PayloadHashVariations 结构体派生正确（Serialize/Debug/Clone/Default）
//      并隐式验证既有 revision_metrics 测试零回归（由 cargo test 全量覆盖）
// ============================================================
#[test]
fn t06_payload_hash_variations_traits_usable() {
    use optimizer_store::PayloadHashVariations;
    let m = PayloadHashVariations {
        total_proposals: 3,
        proposals_with_variation: 2,
        variations: 3,
        variation_rate: 2.0 / 3.0,
    };
    let cloned = m.clone();
    assert_eq!(m, cloned);
    // Debug 派生可用
    let _debug_string = format!("{m:?}");
    // Default 派生可用（冷启动场景）
    let default = PayloadHashVariations::default();
    assert_eq!(default.total_proposals, 0);
    assert_eq!(default.proposals_with_variation, 0);
    assert_eq!(default.variations, 0);
    assert_eq!(default.variation_rate, 0.0);
    // Serialize 派生可用，camelCase 命名
    let json = serde_json::to_value(&m).expect("serde_json::to_value failed");
    assert_eq!(json["totalProposals"], serde_json::json!(3));
    assert_eq!(json["proposalsWithVariation"], serde_json::json!(2));
    assert_eq!(json["variations"], serde_json::json!(3));
    assert!(
        (json["variationRate"].as_f64().unwrap() - 2.0 / 3.0).abs() < f64::EPSILON,
        "variationRate serde mismatch"
    );
}
