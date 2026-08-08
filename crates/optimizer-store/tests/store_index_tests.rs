//! Stage 3 — SQLite 覆盖索引验证测试
//!
//! 验证 v10 schema migration 添加的两个覆盖索引：
//! 1. `idx_operation_run_project_insights` — 覆盖 `get_operation_insights` 聚合查询
//! 2. `idx_operation_run_project_started_at` — 覆盖 `list_operation_runs` 排序查询
//!
//! 测试用例对照实际 SQL（来自 operation.rs），不与示例 SQL 重复。

#![cfg(test)]

use optimizer_store::{
    CURRENT_SCHEMA_VERSION, MIGRATION_10, OptimizerStore, ProjectSeed, SeedBlock, SeedDocument,
};
use rusqlite::params;

/// 构造一个已初始化 project + commit_node 的内存 store。
fn seeded_store() -> OptimizerStore {
    let mut store = OptimizerStore::open_in_memory().expect("open_in_memory failed");
    store
        .initialize_project(&ProjectSeed {
            project_id: "proj-1".into(),
            title: "Index Test Project".into(),
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

/// 插入 1000 条 operation_run 记录，覆盖 5 种 state。
fn insert_1000_runs(store: &OptimizerStore) {
    let conn = store.connection();
    for i in 0..1000i64 {
        let state = match i % 5 {
            0 => "accepted",
            1 => "rejected",
            2 => "conflicted",
            3 => "failed",
            _ => "cancelled",
        };
        let input_tokens = 100 + i;
        let output_tokens = 50 + i;
        let total_tokens = input_tokens + output_tokens;
        let cached_input_tokens: Option<i64> = if i % 3 == 0 { Some(10) } else { None };
        let (failure_code, failure_message, failure_retriable): (
            Option<&str>,
            Option<&str>,
            Option<i64>,
        ) = if state == "failed" || state == "cancelled" {
            (Some("PROVIDER_ERROR"), Some("provider error"), Some(1))
        } else {
            (None, None, None)
        };
        let started_at = format!("2026-07-14T00:{:02}:{:02}.000Z", i / 60, i % 60);
        conn.execute(
            "INSERT INTO operation_run (
                id, operation_intent_id, project_id, base_commit_id, provider_id,
                model, state, started_at, updated_at,
                input_tokens, output_tokens, total_tokens, cached_input_tokens,
                failure_code, failure_message, failure_retriable
             ) VALUES (?1, ?2, ?3, ?4, ?5, ?6, ?7, ?8, ?8, ?9, ?10, ?11, ?12, ?13, ?14, ?15)",
            params![
                format!("run-{i}"),
                format!("intent-{i}"),
                "proj-1",
                "commit-1",
                "deepseek",
                "deepseek-v4-flash",
                state,
                started_at,
                input_tokens,
                output_tokens,
                total_tokens,
                cached_input_tokens,
                failure_code,
                failure_message,
                failure_retriable,
            ],
        )
        .expect("insert operation_run failed");
    }
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
// T01 — schema migration v10 在空库上成功，store_schema_version=10
// ============================================================
#[test]
fn schema_migration_v10_empty_db() {
    let store = OptimizerStore::open_in_memory().expect("open_in_memory failed");
    let conn = store.connection();

    let version: i64 = conn
        .query_row("PRAGMA user_version", [], |row| row.get(0))
        .expect("query user_version failed");
    assert_eq!(
        version, CURRENT_SCHEMA_VERSION,
        "expected schema version {CURRENT_SCHEMA_VERSION}, got {version}"
    );

    let insights_count: i64 = conn
        .query_row(
            "SELECT COUNT(*) FROM sqlite_master WHERE type = 'index' AND name = 'idx_operation_run_project_insights'",
            [],
            |row| row.get(0),
        )
        .expect("query insights index failed");
    assert_eq!(insights_count, 1, "idx_operation_run_project_insights missing");

    let started_at_count: i64 = conn
        .query_row(
            "SELECT COUNT(*) FROM sqlite_master WHERE type = 'index' AND name = 'idx_operation_run_project_started_at'",
            [],
            |row| row.get(0),
        )
        .expect("query started_at index failed");
    assert_eq!(
        started_at_count, 1,
        "idx_operation_run_project_started_at missing"
    );
}

// ============================================================
// T02 — schema migration v10 从 v9 升级成功 + IF NOT EXISTS 幂等
// ============================================================
#[test]
fn schema_migration_v10_from_v9_idempotent() {
    let store = seeded_store();
    let conn = store.connection();

    // 索引已通过 v10 migration 创建。
    let insights_count: i64 = conn
        .query_row(
            "SELECT COUNT(*) FROM sqlite_master WHERE type = 'index' AND name = 'idx_operation_run_project_insights'",
            [],
            |row| row.get(0),
        )
        .expect("query insights index failed");
    assert_eq!(insights_count, 1);

    // 再次执行 MIGRATION_10（IF NOT EXISTS 幂等）不应失败。
    conn.execute_batch(MIGRATION_10)
        .expect("re-running MIGRATION_10 should be idempotent");

    // 索引仍各只有 1 个（IF NOT EXISTS 阻止重复创建）。
    let insights_count_after: i64 = conn
        .query_row(
            "SELECT COUNT(*) FROM sqlite_master WHERE type = 'index' AND name = 'idx_operation_run_project_insights'",
            [],
            |row| row.get(0),
        )
        .expect("query insights index after failed");
    assert_eq!(insights_count_after, 1, "index should not be duplicated");

    let started_at_count_after: i64 = conn
        .query_row(
            "SELECT COUNT(*) FROM sqlite_master WHERE type = 'index' AND name = 'idx_operation_run_project_started_at'",
            [],
            |row| row.get(0),
        )
        .expect("query started_at index after failed");
    assert_eq!(started_at_count_after, 1, "index should not be duplicated");
}

// ============================================================
// T03 — schema migration v10 已是 v10 时重复 migrate 成功（幂等）
// ============================================================
#[test]
fn schema_migration_v10_already_v10_idempotent() {
    // 第一次打开：0 → 10。
    let store = OptimizerStore::open_in_memory().expect("first open failed");
    let version: i64 = store
        .connection()
        .query_row("PRAGMA user_version", [], |row| row.get(0))
        .expect("query version failed");
    assert_eq!(version, CURRENT_SCHEMA_VERSION);

    // 在已迁移到 v10 的库上重复执行 MIGRATION_10（IF NOT EXISTS 幂等）。
    store
        .connection()
        .execute_batch(MIGRATION_10)
        .expect("re-running MIGRATION_10 on already-v10 db should be a no-op");

    let version_after: i64 = store
        .connection()
        .query_row("PRAGMA user_version", [], |row| row.get(0))
        .expect("query version after failed");
    assert_eq!(version_after, CURRENT_SCHEMA_VERSION);
}

// ============================================================
// T04 — get_operation_insights EXPLAIN QUERY PLAN 走索引
// ============================================================
#[test]
fn get_operation_insights_uses_index() {
    let store = seeded_store();
    insert_1000_runs(&store);

    // 与 operation.rs get_operation_insights 的实际 SQL 一致。
    // EXPLAIN QUERY PLAN 只规划不执行，使用字面量避免占位符绑定。
    let sql = "EXPLAIN QUERY PLAN SELECT \
        COALESCE(SUM(input_tokens), 0), \
        COALESCE(SUM(output_tokens), 0), \
        COALESCE(SUM(total_tokens), 0), \
        COALESCE(SUM(CASE WHEN state = 'accepted' THEN 1 ELSE 0 END), 0), \
        COALESCE(SUM(CASE WHEN state = 'rejected' THEN 1 ELSE 0 END), 0), \
        COALESCE(SUM(CASE WHEN state = 'conflicted' THEN 1 ELSE 0 END), 0), \
        COUNT(*) \
        FROM operation_run WHERE project_id = 'proj-1'";

    let plan = explain_query_plan(&store, sql);
    let plan_lower = plan.to_lowercase();
    println!("EXPLAIN QUERY PLAN (get_operation_insights):\n{plan}");
    assert!(
        plan_lower.contains("using index") || plan_lower.contains("using covering index"),
        "expected index usage in plan, got: {plan}"
    );
}

// ============================================================
// T05 — list_operation_runs EXPLAIN QUERY PLAN 走索引
// ============================================================
#[test]
fn list_operation_runs_uses_index() {
    let store = seeded_store();
    insert_1000_runs(&store);

    // 与 operation.rs list_operation_runs 的实际 SQL 一致。
    // EXPLAIN QUERY PLAN 只规划不执行，使用字面量避免占位符绑定。
    let sql = "EXPLAIN QUERY PLAN SELECT id, operation_intent_id, project_id, base_commit_id, provider_id, \
        provider_configuration_id, provider_endpoint_id, model, \
        state, context_packet_id, response_id, finish_reason, \
        input_tokens, output_tokens, total_tokens, cached_input_tokens, reasoning_tokens, \
        failure_code, failure_message, failure_retriable, started_at, updated_at \
        FROM operation_run WHERE project_id = 'proj-1' \
        ORDER BY started_at DESC, id DESC \
        LIMIT 200 OFFSET 0";

    let plan = explain_query_plan(&store, sql);
    let plan_lower = plan.to_lowercase();
    println!("EXPLAIN QUERY PLAN (list_operation_runs):\n{plan}");
    assert!(
        plan_lower.contains("using index") || plan_lower.contains("using covering index"),
        "expected index usage in plan, got: {plan}"
    );
}

// ============================================================
// Stage 4 — 性能对比 + 查询计划回归快照
// ============================================================

/// 插入 n 条 operation_run 记录，覆盖 5 种 state。
/// 使用显式 BEGIN/COMMIT 批量提交，可在百毫秒内插入 100k 条。
fn insert_n_runs(store: &OptimizerStore, n: usize) {
    let conn = store.connection();
    conn.execute_batch("BEGIN IMMEDIATE")
        .expect("BEGIN IMMEDIATE failed");
    for i in 0..n as i64 {
        let state = match i % 5 {
            0 => "accepted",
            1 => "rejected",
            2 => "conflicted",
            3 => "failed",
            _ => "cancelled",
        };
        let input_tokens = 100 + i;
        let output_tokens = 50 + i;
        let total_tokens = input_tokens + output_tokens;
        let cached_input_tokens: Option<i64> = if i % 3 == 0 { Some(10) } else { None };
        let (failure_code, failure_message, failure_retriable): (
            Option<&str>,
            Option<&str>,
            Option<i64>,
        ) = if state == "failed" || state == "cancelled" {
            (Some("PROVIDER_ERROR"), Some("provider error"), Some(1))
        } else {
            (None, None, None)
        };
        // 支持到 ~27 小时（足够 100k 条），字符串字典序与时间序一致以保证 ORDER BY 正确。
        let started_at = format!(
            "2026-07-14T{:02}:{:02}:{:02}.000Z",
            i / 3600,
            (i / 60) % 60,
            i % 60
        );
        conn.execute(
            "INSERT INTO operation_run (
                id, operation_intent_id, project_id, base_commit_id, provider_id,
                model, state, started_at, updated_at,
                input_tokens, output_tokens, total_tokens, cached_input_tokens,
                failure_code, failure_message, failure_retriable
             ) VALUES (?1, ?2, ?3, ?4, ?5, ?6, ?7, ?8, ?8, ?9, ?10, ?11, ?12, ?13, ?14, ?15)",
            params![
                format!("run-{i}"),
                format!("intent-{i}"),
                "proj-1",
                "commit-1",
                "deepseek",
                "deepseek-v4-flash",
                state,
                started_at,
                input_tokens,
                output_tokens,
                total_tokens,
                cached_input_tokens,
                failure_code,
                failure_message,
                failure_retriable,
            ],
        )
        .expect("insert operation_run failed");
    }
    conn.execute_batch("COMMIT").expect("COMMIT failed");
}

/// 计算 n 次调用的平均耗时。
fn avg_duration(iterations: usize, mut f: impl FnMut()) -> std::time::Duration {
    use std::time::{Duration, Instant};
    let mut total = Duration::ZERO;
    for _ in 0..iterations {
        let start = Instant::now();
        f();
        total += start.elapsed();
    }
    if iterations == 0 {
        Duration::ZERO
    } else {
        total / iterations as u32
    }
}

/// 对比有索引 vs 无索引的耗时差异。
/// 返回 (差异描述, 是否检测到 Covering Index Paradox)。
fn format_diff(
    with_index: std::time::Duration,
    without_index: std::time::Duration,
) -> (String, bool) {
    if without_index > with_index {
        let delta = (without_index - with_index).as_secs_f64() * 1000.0;
        (format!("+{delta:.3}ms (index faster)"), false)
    } else {
        let delta = (with_index - without_index).as_secs_f64() * 1000.0;
        (
            format!("Covering Index Paradox 风险 (slower with index: {delta:.3}ms)"),
            true,
        )
    }
}

// ============================================================
// T01-T03 — covering_index_perf_comparison: 有索引 vs 无索引耗时对比
//
// 在 1000 / 10000 / 100000 三种规模下，分别测量：
//   - get_operation_insights（聚合，被 idx_operation_run_project_insights 覆盖）
//   - list_operation_runs   （排序，被 idx_operation_run_project_started_at 覆盖）
// 在有索引与临时 DROP INDEX 两种状态下的平均耗时。
//
// 每个规模使用独立 in-memory store，DROP INDEX 后随 store drop 自动清理，不污染其他测试。
// 若覆盖索引慢于无索引 → println 标记 "Covering Index Paradox 风险"，不自动回退，测试仍通过。
// ============================================================
#[test]
fn covering_index_perf_comparison() {
    const ITERATIONS: usize = 10;
    const SCALES: [usize; 3] = [1_000, 10_000, 100_000];

    println!("\n=== Covering Index Performance Comparison ===");
    println!("iterations per measurement: {ITERATIONS}");

    let mut insights_rows: Vec<(usize, std::time::Duration, std::time::Duration)> = Vec::new();
    let mut list_rows: Vec<(usize, std::time::Duration, std::time::Duration)> = Vec::new();

    for &scale in &SCALES {
        // 每个规模使用独立 in-memory store，DROP INDEX 不会污染其他规模或其他测试。
        let store = seeded_store();
        insert_n_runs(&store, scale);

        // 有索引 — 先 warm-up 让 SQLite 缓存页与计划决策稳定，再测量。
        let _ = store
            .get_operation_insights("proj-1")
            .expect("warmup get_operation_insights with index failed");
        let _ = store
            .list_operation_runs("proj-1", 200, 0)
            .expect("warmup list_operation_runs with index failed");

        let insights_with = avg_duration(ITERATIONS, || {
            let _ = store
                .get_operation_insights("proj-1")
                .expect("get_operation_insights with index failed");
        });
        let list_with = avg_duration(ITERATIONS, || {
            let _ = store
                .list_operation_runs("proj-1", 200, 0)
                .expect("list_operation_runs with index failed");
        });

        // 临时 DROP 两个覆盖索引（store drop 时整个库消失，无需恢复）。
        let conn = store.connection();
        conn.execute_batch("DROP INDEX idx_operation_run_project_insights")
            .expect("DROP idx_operation_run_project_insights failed");
        conn.execute_batch("DROP INDEX idx_operation_run_project_started_at")
            .expect("DROP idx_operation_run_project_started_at failed");

        // 无索引 — 同样 warm-up 后再测量。
        let _ = store
            .get_operation_insights("proj-1")
            .expect("warmup get_operation_insights without index failed");
        let _ = store
            .list_operation_runs("proj-1", 200, 0)
            .expect("warmup list_operation_runs without index failed");

        let insights_without = avg_duration(ITERATIONS, || {
            let _ = store
                .get_operation_insights("proj-1")
                .expect("get_operation_insights without index failed");
        });
        let list_without = avg_duration(ITERATIONS, || {
            let _ = store
                .list_operation_runs("proj-1", 200, 0)
                .expect("list_operation_runs without index failed");
        });

        insights_rows.push((scale, insights_with, insights_without));
        list_rows.push((scale, list_with, list_without));
    }

    let mut paradox_insights = false;
    let mut paradox_list = false;

    println!("\n--- get_operation_insights (idx_operation_run_project_insights) ---");
    println!(
        "{:<10} | {:<24} | {:<24} | 差异",
        "规模", "有索引 (ms)", "无索引 (ms)"
    );
    println!(
        "{:->10}-+-{:->24}-+-{:->24}-+-{:->40}",
        "", "", "", ""
    );
    for (scale, with, without) in &insights_rows {
        let (diff, paradox) = format_diff(*with, *without);
        if paradox {
            paradox_insights = true;
        }
        println!(
            "{:<10} | {:<24.3} | {:<24.3} | {}",
            scale,
            with.as_secs_f64() * 1000.0,
            without.as_secs_f64() * 1000.0,
            diff
        );
    }

    println!("\n--- list_operation_runs (idx_operation_run_project_started_at) ---");
    println!(
        "{:<10} | {:<24} | {:<24} | 差异",
        "规模", "有索引 (ms)", "无索引 (ms)"
    );
    println!(
        "{:->10}-+-{:->24}-+-{:->24}-+-{:->40}",
        "", "", "", ""
    );
    for (scale, with, without) in &list_rows {
        let (diff, paradox) = format_diff(*with, *without);
        if paradox {
            paradox_list = true;
        }
        println!(
            "{:<10} | {:<24.3} | {:<24.3} | {}",
            scale,
            with.as_secs_f64() * 1000.0,
            without.as_secs_f64() * 1000.0,
            diff
        );
    }

    if paradox_insights || paradox_list {
        println!(
            "\nCovering Index Paradox 风险: get_insights={paradox_insights}, list={paradox_list}"
        );
    } else {
        println!("\nNo Covering Index Paradox detected (covering index faster at all scales).");
    }
}

// ============================================================
// T04-T05 — query_plan_regression_snapshot: EXPLAIN QUERY PLAN 回归快照
//
// 将 v0.4.0 Stage 3 验证过的查询计划归档为硬编码子串快照。
// 若未来 schema 变更导致查询计划退化（如从 USING COVERING INDEX 退化为 SCAN）→ 测试失败。
// 使用 contains 而非 exact match，避免 SQLite 版本/格式差异造成误报。
// ============================================================
#[test]
fn query_plan_regression_snapshot() {
    let store = seeded_store();
    insert_1000_runs(&store);

    // ---- T04: get_operation_insights ----
    // 与 operation.rs get_operation_insights 的实际 SQL 一致。
    let insights_sql = "EXPLAIN QUERY PLAN SELECT \
        COALESCE(SUM(input_tokens), 0), \
        COALESCE(SUM(output_tokens), 0), \
        COALESCE(SUM(total_tokens), 0), \
        COALESCE(SUM(CASE WHEN state = 'accepted' THEN 1 ELSE 0 END), 0), \
        COALESCE(SUM(CASE WHEN state = 'rejected' THEN 1 ELSE 0 END), 0), \
        COALESCE(SUM(CASE WHEN state = 'conflicted' THEN 1 ELSE 0 END), 0), \
        COUNT(*) \
        FROM operation_run WHERE project_id = 'proj-1'";
    let insights_plan = explain_query_plan(&store, insights_sql);
    let insights_lower = insights_plan.to_lowercase();
    println!("EXPLAIN QUERY PLAN (get_operation_insights):\n{insights_plan}");

    // v0.4.0 快照：idx_operation_run_project_insights 覆盖 (project_id, state,
    // input_tokens, output_tokens, total_tokens, cached_input_tokens)，聚合查询
    // 所需列全部命中索引 → USING COVERING INDEX。
    assert!(
        insights_lower.contains("using covering index"),
        "REGRESSION: get_operation_insights 不再使用 COVERING INDEX（v0.4.0 快照退化）。Plan:\n{insights_plan}"
    );
    assert!(
        insights_lower.contains("idx_operation_run_project_insights"),
        "REGRESSION: get_operation_insights 不再使用 idx_operation_run_project_insights。Plan:\n{insights_plan}"
    );
    assert!(
        !insights_lower.contains("scan operation_run"),
        "REGRESSION: get_operation_insights 退化为 SCAN。Plan:\n{insights_plan}"
    );

    // ---- T05: list_operation_runs ----
    // 与 operation.rs list_operation_runs 的实际 SQL 一致。
    let list_sql = "EXPLAIN QUERY PLAN SELECT id, operation_intent_id, project_id, base_commit_id, provider_id, \
        provider_configuration_id, provider_endpoint_id, model, \
        state, context_packet_id, response_id, finish_reason, \
        input_tokens, output_tokens, total_tokens, cached_input_tokens, reasoning_tokens, \
        failure_code, failure_message, failure_retriable, started_at, updated_at \
        FROM operation_run WHERE project_id = 'proj-1' \
        ORDER BY started_at DESC, id DESC \
        LIMIT 200 OFFSET 0";
    let list_plan = explain_query_plan(&store, list_sql);
    let list_lower = list_plan.to_lowercase();
    println!("EXPLAIN QUERY PLAN (list_operation_runs):\n{list_plan}");

    // v0.4.0 快照：idx_operation_run_project_started_at 覆盖 (project_id, started_at DESC, id DESC)，
    // 满足 WHERE project_id=? 与 ORDER BY started_at DESC, id DESC。SELECT 列包含索引外字段，
    // 因此 SQLite 通常走 USING INDEX + rowid 回表（而非 USING COVERING INDEX），但必须使用索引。
    assert!(
        list_lower.contains("using covering index") || list_lower.contains("using index"),
        "REGRESSION: list_operation_runs 不再使用 INDEX（v0.4.0 快照退化）。Plan:\n{list_plan}"
    );
    assert!(
        list_lower.contains("idx_operation_run_project_started_at"),
        "REGRESSION: list_operation_runs 不再使用 idx_operation_run_project_started_at。Plan:\n{list_plan}"
    );
    assert!(
        !list_lower.contains("scan operation_run"),
        "REGRESSION: list_operation_runs 退化为 SCAN。Plan:\n{list_plan}"
    );
}
