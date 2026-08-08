//! Stage 2 — 二次编辑率指标计算测试 (v0.6.0)
//!
//! 验证 `get_revision_metrics` store 方法：
//! - `accepted_after_rejection`: 同一 proposal_id 的 patch_review_event 序列中
//!   先出现 Reject 后又出现 Apply 的 proposal 数量
//! - `accepted_after_rejection_rate = accepted_after_rejection / max(total_proposals_accepted, 1)`
//! - `total_proposals_accepted`: 至少有一次 Apply 的 proposal 总数
//! - `proposals_with_rejection`: 至少有一次 Reject 的 proposal 总数
//! - 多次 Reject 后 Apply 只计一次
//!
//! 同时验证 MIGRATION_11 添加的覆盖索引 `idx_patch_review_event_proposal_kind(proposal_id, kind)`
//! 被 EXPLAIN QUERY PLAN 选中。

#![cfg(test)]

use optimizer_store::{
    AppendReviewEvent, MIGRATION_11, OptimizerStore, PersistOperationBundle, ProjectSeed,
    ReviewDecision, ReviewEventKind, ReviewSessionStatus, SeedBlock, SeedDocument,
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
            title: "Revision Metrics Test Project".into(),
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

/// 通过 store API 添加一个 review event，使用自增的 occurred_at 保持顺序。
#[allow(clippy::too_many_arguments)]
fn append_event(
    store: &mut OptimizerStore,
    id: &str,
    proposal_id: &str,
    expected_revision: i64,
    expected_status: ReviewSessionStatus,
    kind: ReviewEventKind,
    next_status: ReviewSessionStatus,
    occurred_at: &str,
) {
    store
        .append_review_event(&AppendReviewEvent {
            id: id.into(),
            proposal_id: proposal_id.into(),
            expected_revision,
            expected_status,
            kind,
            next_status,
            hunk_id: None,
            decision: None,
            payload_json: None,
            occurred_at: occurred_at.into(),
        })
        .expect("append_review_event failed");
}

/// 通过 store API 添加一个 hunk 决策事件（review → ready）。
#[allow(clippy::too_many_arguments)]
fn append_decision(
    store: &mut OptimizerStore,
    id: &str,
    proposal_id: &str,
    expected_revision: i64,
    hunk_id: &str,
    decision: ReviewDecision,
    next_status: ReviewSessionStatus,
    occurred_at: &str,
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
            payload_json: None,
            occurred_at: occurred_at.into(),
        })
        .expect("append_decision failed");
}

/// 收集 EXPLAIN QUERY PLAN 的输出文本。
fn explain_query_plan(store: &OptimizerStore, sql: &str) -> String {
    let conn = store.connection();
    let mut stmt = conn.prepare(sql).expect("prepare EXPLAIN failed");
    let rows: Vec<String> = stmt
        .query_map([], |row| row.get::<_, String>(3))
        .expect("query_map failed")
        .map(|r| r.unwrap_or_default())
        .collect();
    rows.join("\n")
}

// ============================================================
// T01 — 空 project 返回全零 metrics（冷启动）
// ============================================================
#[test]
fn t01_empty_project_returns_zero_metrics() {
    let store = seeded_store();
    let metrics = store
        .get_revision_metrics("proj-1")
        .expect("get_revision_metrics on empty project failed");
    assert_eq!(metrics.total_proposals_accepted, 0);
    assert_eq!(metrics.proposals_with_rejection, 0);
    assert_eq!(metrics.accepted_after_rejection, 0);
    assert_eq!(metrics.accepted_after_rejection_rate, 0.0);
}

// ============================================================
// T02 — 纯接受 proposal（只有 Apply）不计入 accepted_after_rejection
// ============================================================
#[test]
fn t02_pure_accept_not_counted_as_revision() {
    let mut store = seeded_store();
    store
        .persist_operation_bundle(&proposal_bundle(
            "run-accept",
            "ctx-accept",
            "proposal-accept",
        ))
        .expect("persist bundle failed");

    // review → ready (decision/accepted) → applied (apply)
    append_decision(
        &mut store,
        "event-1",
        "proposal-accept",
        0,
        "hunk-1",
        ReviewDecision::Accepted,
        ReviewSessionStatus::Ready,
        "2026-07-15T00:02:00.000Z",
    );
    append_event(
        &mut store,
        "event-2",
        "proposal-accept",
        1,
        ReviewSessionStatus::Ready,
        ReviewEventKind::Apply,
        ReviewSessionStatus::Applied,
        "2026-07-15T00:03:00.000Z",
    );

    let metrics = store
        .get_revision_metrics("proj-1")
        .expect("get_revision_metrics failed");
    assert_eq!(metrics.total_proposals_accepted, 1);
    assert_eq!(metrics.proposals_with_rejection, 0);
    assert_eq!(metrics.accepted_after_rejection, 0);
    assert_eq!(metrics.accepted_after_rejection_rate, 0.0);
}

// ============================================================
// T03 — Reject 后 Apply 计入 accepted_after_rejection
// ============================================================
#[test]
fn t03_reject_then_apply_counted() {
    let mut store = seeded_store();
    store
        .persist_operation_bundle(&proposal_bundle(
            "run-rev",
            "ctx-rev",
            "proposal-rev",
        ))
        .expect("persist bundle failed");

    // review → rejected (reject) ... then directly reject → applied 不可达
    // patch_review_head 状态机：rejected 只能从 review/ready/conflicted 转入。
    // 但 patch_review_event 表是不可变审计日志，可在新 proposal 上记录 reject→apply 序列。
    // 这里走合法路径：review → reject → (operation_run 转 rejected 状态)。
    // 由于 head 状态约束，reject 后无法在同一 proposal 上再 apply。
    // 测试 D2 "不严格定义"：直接通过 SQL 插入 reject 后 apply 的事件序列。
    // 第一条事件：合法的 reject（review → rejected）
    append_event(
        &mut store,
        "event-reject",
        "proposal-rev",
        0,
        ReviewSessionStatus::Review,
        ReviewEventKind::Reject,
        ReviewSessionStatus::Rejected,
        "2026-07-15T00:02:00.000Z",
    );
    // 此时 head.revision=1, status=rejected。要再 apply，必须先把 head 改回 ready，
    // 但 head 状态机不允许 rejected→ready。绕过 trigger 直接插入审计事件以模拟"二次编辑"。
    // 关闭 foreign_keys/trigger 后插入，再恢复，验证聚合逻辑独立于状态机。
    let conn = store.connection();
    conn.execute_batch("PRAGMA foreign_keys = OFF")
        .expect("disable FK failed");
    conn.execute(
        "INSERT INTO patch_review_event(
           id, proposal_id, sequence, base_revision, new_revision, kind,
           previous_status, next_status, hunk_id, decision, payload_json, occurred_at
         ) VALUES (?1, ?2, ?3, ?4, ?5, ?6, ?7, ?8, NULL, NULL, NULL, ?9)",
        params![
            "event-apply-after-reject",
            "proposal-rev",
            2i64,
            1i64,
            2i64,
            "apply",
            "rejected",
            "applied",
            "2026-07-15T00:03:00.000Z",
        ],
    )
    .expect("insert apply event failed");
    conn.execute_batch("PRAGMA foreign_keys = ON")
        .expect("enable FK failed");

    let metrics = store
        .get_revision_metrics("proj-1")
        .expect("get_revision_metrics failed");
    assert_eq!(metrics.total_proposals_accepted, 1);
    assert_eq!(metrics.proposals_with_rejection, 1);
    assert_eq!(metrics.accepted_after_rejection, 1);
    // 1/1 = 1.0
    assert!(
        (metrics.accepted_after_rejection_rate - 1.0).abs() < f64::EPSILON,
        "rate should be 1.0, got {}",
        metrics.accepted_after_rejection_rate
    );
}

// ============================================================
// T04 — 多次 Reject 后 Apply 只计一次
// ============================================================
#[test]
fn t04_multiple_rejects_then_apply_counted_once() {
    let mut store = seeded_store();
    store
        .persist_operation_bundle(&proposal_bundle(
            "run-multi",
            "ctx-multi",
            "proposal-multi",
        ))
        .expect("persist bundle failed");

    // 第 1 次 reject（合法）
    append_event(
        &mut store,
        "event-reject-1",
        "proposal-multi",
        0,
        ReviewSessionStatus::Review,
        ReviewEventKind::Reject,
        ReviewSessionStatus::Rejected,
        "2026-07-15T00:02:00.000Z",
    );

    // 后续 reject → reject → apply 直接 SQL 插入（绕过状态机），模拟 D2 不严格定义。
    let conn = store.connection();
    conn.execute_batch("PRAGMA foreign_keys = OFF")
        .expect("disable FK failed");
    conn.execute(
        "INSERT INTO patch_review_event(
           id, proposal_id, sequence, base_revision, new_revision, kind,
           previous_status, next_status, hunk_id, decision, payload_json, occurred_at
         ) VALUES (?1, ?2, ?3, ?4, ?5, ?6, ?7, ?8, NULL, NULL, NULL, ?9)",
        params![
            "event-reject-2",
            "proposal-multi",
            2i64,
            1i64,
            2i64,
            "reject",
            "rejected",
            "rejected",
            "2026-07-15T00:03:00.000Z",
        ],
    )
    .expect("insert reject-2 failed");
    conn.execute(
        "INSERT INTO patch_review_event(
           id, proposal_id, sequence, base_revision, new_revision, kind,
           previous_status, next_status, hunk_id, decision, payload_json, occurred_at
         ) VALUES (?1, ?2, ?3, ?4, ?5, ?6, ?7, ?8, NULL, NULL, NULL, ?9)",
        params![
            "event-apply-final",
            "proposal-multi",
            3i64,
            2i64,
            3i64,
            "apply",
            "rejected",
            "applied",
            "2026-07-15T00:04:00.000Z",
        ],
    )
    .expect("insert apply failed");
    conn.execute_batch("PRAGMA foreign_keys = ON")
        .expect("enable FK failed");

    let metrics = store
        .get_revision_metrics("proj-1")
        .expect("get_revision_metrics failed");
    assert_eq!(metrics.total_proposals_accepted, 1);
    assert_eq!(metrics.proposals_with_rejection, 1);
    // 多次 reject 后 apply 仍只计 1
    assert_eq!(metrics.accepted_after_rejection, 1);
}

// ============================================================
// T05 — rate 计算正确（冷启动 rate=0.0，多 proposal 比例）
// ============================================================
#[test]
fn t05_rate_calculation_including_cold_start() {
    let mut store = seeded_store();

    // proposal A: 纯接受（apply）
    store
        .persist_operation_bundle(&proposal_bundle("run-a", "ctx-a", "proposal-a"))
        .expect("persist A failed");
    append_decision(
        &mut store,
        "event-a-1",
        "proposal-a",
        0,
        "hunk-a",
        ReviewDecision::Accepted,
        ReviewSessionStatus::Ready,
        "2026-07-15T00:02:00.000Z",
    );
    append_event(
        &mut store,
        "event-a-2",
        "proposal-a",
        1,
        ReviewSessionStatus::Ready,
        ReviewEventKind::Apply,
        ReviewSessionStatus::Applied,
        "2026-07-15T00:03:00.000Z",
    );

    // proposal B: reject → apply（绕过状态机）
    store
        .persist_operation_bundle(&proposal_bundle("run-b", "ctx-b", "proposal-b"))
        .expect("persist B failed");
    append_event(
        &mut store,
        "event-b-reject",
        "proposal-b",
        0,
        ReviewSessionStatus::Review,
        ReviewEventKind::Reject,
        ReviewSessionStatus::Rejected,
        "2026-07-15T00:02:00.000Z",
    );

    // proposal C: 只有 reject（未接受）
    store
        .persist_operation_bundle(&proposal_bundle("run-c", "ctx-c", "proposal-c"))
        .expect("persist C failed");
    append_event(
        &mut store,
        "event-c-reject",
        "proposal-c",
        0,
        ReviewSessionStatus::Review,
        ReviewEventKind::Reject,
        ReviewSessionStatus::Rejected,
        "2026-07-15T00:02:00.000Z",
    );

    // 为 proposal B 绕过状态机插入 apply 事件（reject → apply）
    let conn = store.connection();
    conn.execute_batch("PRAGMA foreign_keys = OFF")
        .expect("disable FK failed");
    conn.execute(
        "INSERT INTO patch_review_event(
           id, proposal_id, sequence, base_revision, new_revision, kind,
           previous_status, next_status, hunk_id, decision, payload_json, occurred_at
         ) VALUES (?1, ?2, ?3, ?4, ?5, ?6, ?7, ?8, NULL, NULL, NULL, ?9)",
        params![
            "event-b-apply",
            "proposal-b",
            2i64,
            1i64,
            2i64,
            "apply",
            "rejected",
            "applied",
            "2026-07-15T00:03:00.000Z",
        ],
    )
    .expect("insert apply failed");
    conn.execute_batch("PRAGMA foreign_keys = ON")
        .expect("enable FK failed");
    let _ = conn;

    let metrics = store
        .get_revision_metrics("proj-1")
        .expect("get_revision_metrics failed");
    // total_proposals_accepted: A + B = 2 (C 仅有 reject)
    assert_eq!(metrics.total_proposals_accepted, 2);
    // proposals_with_rejection: B + C = 2
    assert_eq!(metrics.proposals_with_rejection, 2);
    // accepted_after_rejection: B = 1
    assert_eq!(metrics.accepted_after_rejection, 1);
    // rate = 1 / max(2, 1) = 0.5
    assert!(
        (metrics.accepted_after_rejection_rate - 0.5).abs() < f64::EPSILON,
        "rate should be 0.5, got {}",
        metrics.accepted_after_rejection_rate
    );

    // 冷启动场景：在另一个空 project 上 rate 必须为 0.0
    let metrics_empty = store
        .get_revision_metrics("proj-empty")
        .expect("get_revision_metrics on empty project failed");
    assert_eq!(metrics_empty.total_proposals_accepted, 0);
    assert_eq!(metrics_empty.accepted_after_rejection, 0);
    assert_eq!(metrics_empty.accepted_after_rejection_rate, 0.0);
}

// ============================================================
// T06 — MIGRATION_11 幂等（重复执行不报错）
// ============================================================
#[test]
fn t06_migration_11_idempotent() {
    let store = seeded_store();
    let conn = store.connection();

    // 索引已通过 v11 migration 创建。
    let count_before: i64 = conn
        .query_row(
            "SELECT COUNT(*) FROM sqlite_master WHERE type = 'index' AND name = 'idx_patch_review_event_proposal_kind'",
            [],
            |row| row.get(0),
        )
        .expect("query index failed");
    assert_eq!(count_before, 1, "idx_patch_review_event_proposal_kind missing");

    // 再次执行 MIGRATION_11（IF NOT EXISTS 幂等）不应失败。
    conn.execute_batch(MIGRATION_11)
        .expect("re-running MIGRATION_11 should be idempotent");

    // 索引仍只有 1 个（IF NOT EXISTS 阻止重复创建）。
    let count_after: i64 = conn
        .query_row(
            "SELECT COUNT(*) FROM sqlite_master WHERE type = 'index' AND name = 'idx_patch_review_event_proposal_kind'",
            [],
            |row| row.get(0),
        )
        .expect("query index after failed");
    assert_eq!(count_after, 1, "index should not be duplicated");
}

// ============================================================
// T07 — EXPLAIN QUERY PLAN 使用 idx_patch_review_event_proposal_kind 索引
// ============================================================
#[test]
fn t07_aggregation_query_uses_index() {
    let mut store = seeded_store();

    // 准备数据：1 个 proposal 走 reject → apply
    store
        .persist_operation_bundle(&proposal_bundle(
            "run-plan",
            "ctx-plan",
            "proposal-plan",
        ))
        .expect("persist bundle failed");
    append_event(
        &mut store,
        "event-plan-reject",
        "proposal-plan",
        0,
        ReviewSessionStatus::Review,
        ReviewEventKind::Reject,
        ReviewSessionStatus::Rejected,
        "2026-07-15T00:02:00.000Z",
    );
    let conn = store.connection();
    conn.execute_batch("PRAGMA foreign_keys = OFF")
        .expect("disable FK failed");
    conn.execute(
        "INSERT INTO patch_review_event(
           id, proposal_id, sequence, base_revision, new_revision, kind,
           previous_status, next_status, hunk_id, decision, payload_json, occurred_at
         ) VALUES (?1, ?2, ?3, ?4, ?5, ?6, ?7, ?8, NULL, NULL, NULL, ?9)",
        params![
            "event-plan-apply",
            "proposal-plan",
            2i64,
            1i64,
            2i64,
            "apply",
            "rejected",
            "applied",
            "2026-07-15T00:03:00.000Z",
        ],
    )
    .expect("insert apply failed");
    conn.execute_batch("PRAGMA foreign_keys = ON")
        .expect("enable FK failed");

    // 与 operation.rs get_revision_metrics 的实际 SQL 一致：INDEXED BY 显式锁定索引。
    // idx_patch_review_event_proposal_kind(proposal_id, kind) 是为该查询设计的覆盖索引。
    let sql = "EXPLAIN QUERY PLAN SELECT pre.proposal_id, pre.kind, pre.sequence \
        FROM patch_review_event AS pre INDEXED BY idx_patch_review_event_proposal_kind \
        WHERE pre.proposal_id IN ( \
            SELECT h.proposal_id FROM patch_review_head AS h \
            JOIN operation_run AS r ON r.id = h.run_id \
            WHERE r.project_id = 'proj-1' \
        ) \
        AND pre.kind IN ('apply', 'reject')";

    let plan = explain_query_plan(&store, sql);
    let plan_lower = plan.to_lowercase();
    println!("EXPLAIN QUERY PLAN (get_revision_metrics):\n{plan}");

    // 必须使用索引（不能 SCAN）。
    assert!(
        plan_lower.contains("using covering index") || plan_lower.contains("using index"),
        "REGRESSION: get_revision_metrics 不使用 INDEX。Plan:\n{plan}"
    );
    assert!(
        plan_lower.contains("idx_patch_review_event_proposal_kind"),
        "REGRESSION: get_revision_metrics 不使用 idx_patch_review_event_proposal_kind。Plan:\n{plan}"
    );
    assert!(
        !plan_lower.contains("scan patch_review_event"),
        "REGRESSION: get_revision_metrics 退化为 SCAN。Plan:\n{plan}"
    );
}

// ============================================================
// T08 — 既有 store_index_tests + operation 测试零回归（隐式验证）
// ============================================================
// 该测试本身不引入新逻辑：cargo test --workspace 全量执行即覆盖 T08。
// 这里仅校验 RevisionMetrics 结构体派生正确，可在 store 之外被序列化比较。
#[test]
fn t08_revision_metrics_traits_usable() {
    use optimizer_store::RevisionMetrics;
    let m = RevisionMetrics {
        total_proposals_accepted: 2,
        proposals_with_rejection: 1,
        accepted_after_rejection: 1,
        accepted_after_rejection_rate: 0.5,
    };
    let cloned = m.clone();
    assert_eq!(m, cloned);
    // Debug 派生可用
    let _debug_string = format!("{m:?}");
    // Serialize 派生可用（不 panic 即可）
    let json = serde_json::to_value(&m).expect("serde_json::to_value failed");
    assert_eq!(json["totalProposalsAccepted"], serde_json::json!(2));
    assert_eq!(json["proposalsWithRejection"], serde_json::json!(1));
    assert_eq!(json["acceptedAfterRejection"], serde_json::json!(1));
    assert_eq!(json["acceptedAfterRejectionRate"], serde_json::json!(0.5));
}
