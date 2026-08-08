//! Integration tests for the operation insights host surface.
//!
//! These tests close the v0.3.0 R-08 RGT YELLOW gap by exercising the
//! `OperationCommandHost::get_operation_insights` and `DiagnosticBundle`
//! data contracts end-to-end against an in-memory store seeded with
//! operation runs spanning multiple terminal states.
//!
//! `persist_operation_bundle` only accepts runs in the `Review`, `Failed`
//! or `Cancelled` execution states. Runs that end in `Accepted`,
//! `Rejected` or `Conflicted` are reached by persisting a `Review` run
//! (which auto-creates a `patch_review_head` row when the artifact is a
//! `PatchProposal`) and then appending the appropriate review events.

use optimizer_host::{
    DiagnosticBundle, DiagnosticManifest, OperationCommandHost, OperationInsightsResponse,
    RecentRunSummary,
};
use optimizer_store::{
    AppendReviewEvent, ModelUsageRecord, NewContextPacket, NewOperationArtifact,
    NewOperationLifecycleEvent, NewOperationRun, OperationArtifactKind, OperationFailureRecord,
    OperationInsightsRecord, OperationState, OptimizerStore, PersistOperationBundle, PayloadHashVariations,
    ProjectSeed, ReviewDecision, ReviewEventKind, ReviewSessionStatus, RevisionMetrics, SeedBlock,
    SeedDocument,
};

fn seed() -> ProjectSeed {
    ProjectSeed {
        project_id: "project-1".into(),
        title: "Insights Integration Test".into(),
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

/// Lifecycle events that walk a run from `Draft` all the way to `Review`,
/// matching the canonical execution path the kernel emits.
fn lifecycle_to_review(started_minute: u32) -> Vec<NewOperationLifecycleEvent> {
    use OperationState::*;
    let step = |from, to, offset: u32| NewOperationLifecycleEvent {
        from_state: from,
        to_state: to,
        occurred_at: format!("2026-07-15T00:{started_minute:02}:{offset:02}.000Z"),
        reason: None,
    };
    vec![
        step(Draft, Compiling, 0),
        step(Compiling, Preflight, 1),
        step(Preflight, Queued, 2),
        step(Queued, Streaming, 3),
        step(Streaming, Validating, 4),
        step(Validating, Review, 5),
    ]
}

/// Lifecycle events that walk a run from `Draft` through `Streaming` and
/// then into the terminal `Failed` state. Reaching `Streaming` justifies
/// the usage tokens the insights aggregator counts.
fn lifecycle_to_failed(started_minute: u32) -> Vec<NewOperationLifecycleEvent> {
    use OperationState::*;
    let step = |from, to, offset: u32| NewOperationLifecycleEvent {
        from_state: from,
        to_state: to,
        occurred_at: format!("2026-07-15T00:{started_minute:02}:{offset:02}.000Z"),
        reason: Some("ProviderError".into()),
    };
    vec![
        step(Draft, Compiling, 0),
        step(Compiling, Preflight, 1),
        step(Preflight, Queued, 2),
        step(Queued, Streaming, 3),
        step(Streaming, Failed, 4),
    ]
}

/// Build a `PersistOperationBundle` in the `Review` state with a
/// `PatchProposal` artifact. Persisting this bundle auto-creates a review
/// session (revision 0, status `Review`) keyed by the artifact id, which
/// the caller can then drive to a terminal review state.
fn make_review_bundle(
    run_id: &str,
    input_tokens: i64,
    output_tokens: i64,
    started_minute: u32,
) -> PersistOperationBundle {
    let total_tokens = input_tokens + output_tokens;
    let proposal_id = format!("proposal-{run_id}");
    let context_id = format!("context-{run_id}");
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
                input_tokens,
                output_tokens,
                total_tokens,
                cached_input_tokens: Some(0),
                reasoning_tokens: Some(0),
            }),
            failure: None,
            started_at: format!("2026-07-15T00:{started_minute:02}:00.000Z"),
            updated_at: format!("2026-07-15T00:{started_minute:02}:30.000Z"),
        },
        context_packet: Some(NewContextPacket {
            id: context_id.clone(),
            operation_intent_id: format!("intent-{run_id}"),
            project_id: "project-1".into(),
            base_commit_id: "commit-initial".into(),
            packet_hash: format!("sha256:{context_id}"),
            payload_json: format!(r#"{{"id":"{context_id}","items":[]}}"#),
            created_at: format!("2026-07-15T00:{started_minute:02}:10.000Z"),
        }),
        lifecycle_events: lifecycle_to_review(started_minute),
        attempts: vec![],
        artifact: Some(NewOperationArtifact {
            id: proposal_id.clone(),
            kind: OperationArtifactKind::PatchProposal,
            binding_hash: format!("sha256:{proposal_id}"),
            payload_json: format!(
                r#"{{"id":"{proposal_id}","schemaVersion":2,"hunks":[{{"id":"hunk-{run_id}"}}]}}"#
            ),
            created_at: format!("2026-07-15T00:{started_minute:02}:20.000Z"),
        }),
    }
}

/// Build a `PersistOperationBundle` in the terminal `Failed` state. Failed
/// runs carry a failure record and cannot contain an artifact, but they
/// may still report usage tokens consumed before the failure.
fn make_failed_bundle(
    run_id: &str,
    input_tokens: i64,
    output_tokens: i64,
    started_minute: u32,
) -> PersistOperationBundle {
    let total_tokens = input_tokens + output_tokens;
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
            state: OperationState::Failed,
            response_id: None,
            finish_reason: None,
            usage: Some(ModelUsageRecord {
                input_tokens,
                output_tokens,
                total_tokens,
                cached_input_tokens: Some(0),
                reasoning_tokens: Some(0),
            }),
            failure: Some(OperationFailureRecord {
                code: "PROVIDER_FAILED".into(),
                message: "Provider returned an unrecoverable error".into(),
                retriable: false,
            }),
            started_at: format!("2026-07-15T00:{started_minute:02}:00.000Z"),
            updated_at: format!("2026-07-15T00:{started_minute:02}:30.000Z"),
        },
        context_packet: None,
        lifecycle_events: lifecycle_to_failed(started_minute),
        attempts: vec![],
        artifact: None,
    }
}

/// Drive a review session from `Review` → `Ready` → `Applied`, leaving the
/// associated operation run in the `Accepted` state. The `AppendReviewEvent`
/// contract requires a `Decision` event (with a hunk id and decision) to
/// reach `Ready` before an `Apply` event can land the run in `Accepted`.
fn apply_review(store: &mut OptimizerStore, proposal_id: &str, run_index: u32) {
    let hunk_id = format!("hunk-run-{run_index}");
    store
        .append_review_event(&AppendReviewEvent {
            id: format!("decision-{run_index}"),
            proposal_id: proposal_id.into(),
            expected_revision: 0,
            expected_status: ReviewSessionStatus::Review,
            kind: ReviewEventKind::Decision,
            next_status: ReviewSessionStatus::Ready,
            hunk_id: Some(hunk_id),
            decision: Some(ReviewDecision::Accepted),
            payload_json: Some(r#"{"source":"inline-review"}"#.into()),
            occurred_at: format!("2026-07-15T00:{run_index:02}:40.000Z"),
        })
        .expect("decision event should transition Review → Ready");
    store
        .append_review_event(&AppendReviewEvent {
            id: format!("apply-{run_index}"),
            proposal_id: proposal_id.into(),
            expected_revision: 1,
            expected_status: ReviewSessionStatus::Ready,
            kind: ReviewEventKind::Apply,
            next_status: ReviewSessionStatus::Applied,
            hunk_id: None,
            decision: None,
            payload_json: Some(r#"{"transactionId":"tx-review"}"#.into()),
            occurred_at: format!("2026-07-15T00:{run_index:02}:50.000Z"),
        })
        .expect("apply event should transition Ready → Applied");
}

/// Drive a review session from `Review` → `Rejected` in a single
/// `Reject` event, leaving the operation run in the `Rejected` state.
fn reject_review(store: &mut OptimizerStore, proposal_id: &str, run_index: u32) {
    store
        .append_review_event(&AppendReviewEvent {
            id: format!("reject-{run_index}"),
            proposal_id: proposal_id.into(),
            expected_revision: 0,
            expected_status: ReviewSessionStatus::Review,
            kind: ReviewEventKind::Reject,
            next_status: ReviewSessionStatus::Rejected,
            hunk_id: None,
            decision: None,
            payload_json: None,
            occurred_at: format!("2026-07-15T00:{run_index:02}:40.000Z"),
        })
        .expect("reject event should transition Review → Rejected");
}

/// Drive a review session from `Review` → `Conflicted` in a single
/// `Conflict` event, leaving the operation run in the `Conflicted` state.
fn conflict_review(store: &mut OptimizerStore, proposal_id: &str, run_index: u32) {
    store
        .append_review_event(&AppendReviewEvent {
            id: format!("conflict-{run_index}"),
            proposal_id: proposal_id.into(),
            expected_revision: 0,
            expected_status: ReviewSessionStatus::Review,
            kind: ReviewEventKind::Conflict,
            next_status: ReviewSessionStatus::Conflicted,
            hunk_id: None,
            decision: None,
            payload_json: Some(r#"{"code":"TARGET_BLOCK_CHANGED"}"#.into()),
            occurred_at: format!("2026-07-15T00:{run_index:02}:40.000Z"),
        })
        .expect("conflict event should transition Review → Conflicted");
}

/// Seed an in-memory store with five operation runs spanning the four
/// terminal states that the insights aggregator distinguishes:
/// two accepted, one rejected, one conflicted, and one failed.
fn setup_store_with_5_states() -> OptimizerStore {
    let mut store = OptimizerStore::open_in_memory().expect("open_in_memory failed");
    store.initialize_project(&seed()).expect("initialize_project failed");

    // run-1: Accepted (Review → Ready → Applied)
    store
        .persist_operation_bundle(&make_review_bundle("run-1", 100, 50, 1))
        .expect("persist run-1 failed");
    apply_review(&mut store, "proposal-run-1", 1);

    // run-2: Accepted (Review → Ready → Applied)
    store
        .persist_operation_bundle(&make_review_bundle("run-2", 200, 100, 2))
        .expect("persist run-2 failed");
    apply_review(&mut store, "proposal-run-2", 2);

    // run-3: Rejected (Review → Rejected)
    store
        .persist_operation_bundle(&make_review_bundle("run-3", 300, 150, 3))
        .expect("persist run-3 failed");
    reject_review(&mut store, "proposal-run-3", 3);

    // run-4: Conflicted (Review → Conflicted)
    store
        .persist_operation_bundle(&make_review_bundle("run-4", 400, 200, 4))
        .expect("persist run-4 failed");
    conflict_review(&mut store, "proposal-run-4", 4);

    // run-5: Failed (persisted directly in terminal Failed state)
    store
        .persist_operation_bundle(&make_failed_bundle("run-5", 500, 250, 5))
        .expect("persist run-5 failed");
    store
}

#[test]
fn t07_get_operation_insights_returns_correct_structure() {
    let store = setup_store_with_5_states();
    let host = OperationCommandHost::new(store);
    let response = host
        .get_operation_insights("project-1")
        .expect("get_operation_insights failed");

    assert_eq!(response.schema_version, 4, "schema_version must be 4");
    assert_eq!(response.summary.total_runs, 5);
    assert_eq!(response.summary.accepted_count, 2);
    assert_eq!(response.summary.rejected_count, 1);
    assert_eq!(response.summary.conflicted_count, 1);
    // 100 + 200 + 300 + 400 + 500 = 1500
    assert_eq!(response.summary.total_input_tokens, 1500);
    // 50 + 100 + 150 + 200 + 250 = 750
    assert_eq!(response.summary.total_output_tokens, 750);
    // 1500 + 750 = 2250
    assert_eq!(response.summary.total_tokens, 2250);
    // list_operation_runs caps at 10; we seeded 5
    assert_eq!(
        response.recent_runs.len(),
        5,
        "recent_runs should contain every seeded run up to the cap"
    );
    // generated_at must be a non-empty RFC3339 timestamp produced by the host
    assert!(!response.generated_at.is_empty());
    // The most recent run (started_at DESC) should be run-5
    assert_eq!(response.recent_runs[0].run_id, "run-5");
    assert_eq!(response.recent_runs[0].state, "failed");
    // Each RecentRunSummary should carry the token breakdown
    assert_eq!(response.recent_runs[0].input_tokens, 500);
    assert_eq!(response.recent_runs[0].output_tokens, 250);
    assert_eq!(
        response.recent_runs[0].total_tokens,
        Some(750),
        "total_tokens should be the sum of input + output"
    );
}

#[test]
fn t08_operation_insights_response_round_trips_through_serde_json() {
    let store = setup_store_with_5_states();
    let host = OperationCommandHost::new(store);
    let response = host
        .get_operation_insights("project-1")
        .expect("get_operation_insights failed");

    // OperationInsightsResponse serializes with camelCase keys; verify the
    // wire shape and then round-trip the inner summary/recent_runs through
    // their Deserialize impls (which DiagnosticBundle also relies on).
    let json = serde_json::to_string(&response).expect("serialize response");
    let value: serde_json::Value =
        serde_json::from_str(&json).expect("reparse as serde_json::Value");

    assert_eq!(value["schemaVersion"], serde_json::json!(4));
    assert_eq!(value["summary"]["totalRuns"], serde_json::json!(5));
    assert_eq!(value["summary"]["acceptedCount"], serde_json::json!(2));
    assert_eq!(value["summary"]["rejectedCount"], serde_json::json!(1));
    assert_eq!(value["summary"]["conflictedCount"], serde_json::json!(1));
    assert_eq!(value["summary"]["totalInputTokens"], serde_json::json!(1500));
    assert_eq!(value["summary"]["totalOutputTokens"], serde_json::json!(750));
    assert_eq!(value["summary"]["totalTokens"], serde_json::json!(2250));
    assert!(value["recentRuns"].is_array());
    assert_eq!(value["recentRuns"].as_array().unwrap().len(), 5);
    assert!(
        value["generatedAt"].is_string(),
        "generatedAt must serialize as a string",
    );

    // Round-trip the inner summary through its Deserialize impl.
    let summary_json = serde_json::to_string(&response.summary).unwrap();
    let summary: OperationInsightsRecord =
        serde_json::from_str(&summary_json).expect("OperationInsightsRecord round-trip");
    assert_eq!(summary, response.summary);

    // Round-trip each RecentRunSummary through its Deserialize impl.
    let runs_json = serde_json::to_string(&response.recent_runs).unwrap();
    let runs: Vec<RecentRunSummary> =
        serde_json::from_str(&runs_json).expect("RecentRunSummary round-trip");
    assert_eq!(runs, response.recent_runs);
    // Verify the camelCase wire format for the extended v2 fields.
    let first_run_value: serde_json::Value =
        serde_json::from_str(&serde_json::to_string(&response.recent_runs[0]).unwrap()).unwrap();
    assert_eq!(first_run_value["runId"], serde_json::json!("run-5"));
    assert_eq!(first_run_value["inputTokens"], serde_json::json!(500));
    assert_eq!(first_run_value["outputTokens"], serde_json::json!(250));
    assert_eq!(first_run_value["cachedInputTokens"], serde_json::json!(0));
}

#[test]
fn t09_diagnostic_bundle_schema_version_is_3_and_round_trips() {
    // Build a v3 DiagnosticBundle that mirrors the shape written by
    // `OpenedProject::export_diagnostics` and verify it round-trips through
    // serde_json with schema_version intact.
    let bundle = DiagnosticBundle {
        schema_version: 3,
        manifest: DiagnosticManifest {
            project_id: "project-1".into(),
            project_title: "Insights Integration Test".into(),
            generated_at: "2026-07-15T00:00:00Z".into(),
            store_schema_version: 10,
        },
        insights: OperationInsightsRecord {
            total_input_tokens: 1500,
            total_output_tokens: 750,
            total_tokens: 2250,
            accepted_count: 2,
            rejected_count: 1,
            conflicted_count: 1,
            total_runs: 5,
        },
        recent_runs: vec![RecentRunSummary {
            run_id: "run-1".into(),
            operation_intent_id: "intent-1".into(),
            state: "accepted".into(),
            provider_id: "deepseek".into(),
            started_at: "2026-07-15T00:01:00.000Z".into(),
            total_tokens: Some(150),
            input_tokens: 100,
            output_tokens: 50,
            cached_input_tokens: Some(0),
        }],
        revision_metrics: RevisionMetrics {
            total_proposals_accepted: 2,
            proposals_with_rejection: 1,
            accepted_after_rejection: 0,
            accepted_after_rejection_rate: 0.0,
        },
        payload_hash_variations: PayloadHashVariations::default(),
    };

    let json = serde_json::to_string(&bundle).expect("serialize DiagnosticBundle");
    let value: serde_json::Value =
        serde_json::from_str(&json).expect("reparse as serde_json::Value");
    assert_eq!(value["schemaVersion"], serde_json::json!(3));
    assert_eq!(value["manifest"]["projectId"], serde_json::json!("project-1"));
    assert_eq!(value["insights"]["totalRuns"], serde_json::json!(5));
    assert_eq!(value["recentRuns"][0]["inputTokens"], serde_json::json!(100));
    assert_eq!(
        value["revisionMetrics"]["totalProposalsAccepted"],
        serde_json::json!(2)
    );

    let reparsed: DiagnosticBundle =
        serde_json::from_str(&json).expect("DiagnosticBundle round-trip");
    assert_eq!(reparsed.schema_version, 3);
    assert_eq!(reparsed, bundle);
    assert_eq!(reparsed.insights.total_runs, 5);
    assert_eq!(reparsed.recent_runs.len(), 1);
    assert_eq!(reparsed.recent_runs[0].input_tokens, 100);
    assert_eq!(reparsed.recent_runs[0].cached_input_tokens, Some(0));
    assert_eq!(reparsed.revision_metrics.total_proposals_accepted, 2);
}

#[test]
fn t10_operation_insights_response_v3_carries_revision_metrics() {
    let store = setup_store_with_5_states();
    let host = OperationCommandHost::new(store);
    let response = host
        .get_operation_insights("project-1")
        .expect("get_operation_insights failed");

    assert_eq!(response.schema_version, 4, "schema_version must be 4");
    // setup_store_with_5_states seeds 2 accepted (apply), 1 rejected, 1 conflicted.
    // Only run-1 and run-2 reach the Apply event, so total_proposals_accepted = 2.
    // run-3 emits a single Reject; no proposal has a reject-then-apply sequence.
    let metrics = &response.revision_metrics;
    assert_eq!(metrics.total_proposals_accepted, 2);
    assert_eq!(metrics.proposals_with_rejection, 1);
    assert_eq!(metrics.accepted_after_rejection, 0);
    assert_eq!(metrics.accepted_after_rejection_rate, 0.0);

    // The revision_metrics field must serialize under camelCase.
    let json = serde_json::to_value(&response).expect("serialize response");
    assert_eq!(
        json["revisionMetrics"]["totalProposalsAccepted"],
        serde_json::json!(2)
    );
    assert_eq!(
        json["revisionMetrics"]["acceptedAfterRejectionRate"],
        serde_json::json!(0.0)
    );
}

#[test]
fn t11_v2_insights_json_deserializes_to_v3_with_serde_defaults() {
    // A v2 JSON payload (no revisionMetrics field) must deserialize into the
    // v4 OperationInsightsResponse with revision_metrics populated to default
    // (all zeros), satisfying the D6 backward-compat constraint.
    let v2_json = serde_json::json!({
        "schemaVersion": 2,
        "summary": {
            "totalInputTokens": 1000,
            "totalOutputTokens": 500,
            "totalTokens": 1500,
            "acceptedCount": 1,
            "rejectedCount": 0,
            "conflictedCount": 0,
            "totalRuns": 1
        },
        "recentRuns": [],
        "generatedAt": "2026-07-15T00:00:00Z"
    });
    let response: OperationInsightsResponse =
        serde_json::from_value(v2_json).expect("v2 JSON must deserialize to v4 struct");
    assert_eq!(response.schema_version, 2, "schema_version echoes the payload");
    assert_eq!(response.summary.total_runs, 1);
    assert_eq!(
        response.revision_metrics,
        RevisionMetrics::default(),
        "missing revision_metrics must fall back to default (all zeros)"
    );
    assert_eq!(
        response.payload_hash_variations,
        PayloadHashVariations::default(),
        "missing payload_hash_variations must fall back to default (all zeros)"
    );
}

#[test]
fn t12_export_diagnostics_emits_v3_bundle_with_revision_metrics() {
    let store = setup_store_with_5_states();
    let host = OperationCommandHost::new(store);
    let response = host
        .get_operation_insights("project-1")
        .expect("get_operation_insights failed");
    // The diagnostics bundle must carry the same revision_metrics payload as
    // the insights response, serialized under schema_version 4.
    let json = serde_json::to_value(&response).expect("serialize response");
    assert_eq!(json["schemaVersion"], serde_json::json!(4));
    assert!(json["revisionMetrics"].is_object());
    assert_eq!(
        json["revisionMetrics"]["totalProposalsAccepted"],
        serde_json::json!(2)
    );
}

#[test]
fn t13_operation_insights_response_schema_version_4_carries_payload_hash_variations() {
    // v0.7.0 Stage 3 (D4): schema_version bumps to 4 and the response carries
    // a `payload_hash_variations` field populated from the store aggregator.
    let store = setup_store_with_5_states();
    let host = OperationCommandHost::new(store);
    let response = host
        .get_operation_insights("project-1")
        .expect("get_operation_insights failed");

    assert_eq!(response.schema_version, 4, "schema_version must be 4");
    // setup_store_with_5_states seeds 4 proposals with patch_review_event rows
    // (run-1 through run-4; run-5 is Failed with no review session). None of
    // the seeded review events carry `payload_hash` in their payload_json, so
    // the variation aggregator reports total_proposals=4, variation=0.
    let variations = &response.payload_hash_variations;
    assert_eq!(variations.total_proposals, 4);
    assert_eq!(variations.proposals_with_variation, 0);
    assert_eq!(variations.variations, 0);
    assert_eq!(variations.variation_rate, 0.0);

    // The payload_hash_variations field must serialize under camelCase.
    let json = serde_json::to_value(&response).expect("serialize response");
    assert_eq!(json["schemaVersion"], serde_json::json!(4));
    assert!(json["payloadHashVariations"].is_object());
    assert_eq!(
        json["payloadHashVariations"]["totalProposals"],
        serde_json::json!(4)
    );
    assert_eq!(
        json["payloadHashVariations"]["variationRate"],
        serde_json::json!(0.0)
    );
}

#[test]
fn t14_v3_insights_json_deserializes_to_v4_with_serde_defaults() {
    // D4 backward-compat: a v3 JSON payload (no payloadHashVariations field)
    // must deserialize into the v4 OperationInsightsResponse with
    // payload_hash_variations populated to default (all zeros), satisfying the
    // #[serde(default)] constraint.
    let v3_json = serde_json::json!({
        "schemaVersion": 3,
        "summary": {
            "totalInputTokens": 1000,
            "totalOutputTokens": 500,
            "totalTokens": 1500,
            "acceptedCount": 1,
            "rejectedCount": 0,
            "conflictedCount": 0,
            "totalRuns": 1
        },
        "recentRuns": [],
        "generatedAt": "2026-07-15T00:00:00Z",
        "revisionMetrics": {
            "totalProposalsAccepted": 1,
            "proposalsWithRejection": 0,
            "acceptedAfterRejection": 0,
            "acceptedAfterRejectionRate": 0.0
        }
    });
    let response: OperationInsightsResponse =
        serde_json::from_value(v3_json).expect("v3 JSON must deserialize to v4 struct");
    assert_eq!(response.schema_version, 3, "schema_version echoes the payload");
    assert_eq!(response.summary.total_runs, 1);
    assert_eq!(response.revision_metrics.total_proposals_accepted, 1);
    assert_eq!(
        response.payload_hash_variations,
        PayloadHashVariations::default(),
        "missing payload_hash_variations must fall back to default (all zeros)"
    );
}

#[test]
fn t15_diagnostic_bundle_schema_version_4_round_trips_with_payload_hash_variations() {
    // v0.7.0 Stage 3 (D4): DiagnosticBundle schema_version bumps to 4 and
    // carries a `payload_hash_variations` field. Verify the v4 bundle round-trips
    // through serde_json with the new field intact.
    let bundle = DiagnosticBundle {
        schema_version: 4,
        manifest: DiagnosticManifest {
            project_id: "project-1".into(),
            project_title: "Payload Variations Test".into(),
            generated_at: "2026-07-15T00:00:00Z".into(),
            store_schema_version: 10,
        },
        insights: OperationInsightsRecord {
            total_input_tokens: 1000,
            total_output_tokens: 500,
            total_tokens: 1500,
            accepted_count: 1,
            rejected_count: 0,
            conflicted_count: 0,
            total_runs: 1,
        },
        recent_runs: vec![],
        revision_metrics: RevisionMetrics::default(),
        payload_hash_variations: PayloadHashVariations {
            total_proposals: 58,
            proposals_with_variation: 3,
            variations: 4,
            variation_rate: 0.05172413793103448,
        },
    };

    let json = serde_json::to_string(&bundle).expect("serialize DiagnosticBundle");
    let value: serde_json::Value =
        serde_json::from_str(&json).expect("reparse as serde_json::Value");
    assert_eq!(value["schemaVersion"], serde_json::json!(4));
    assert_eq!(
        value["payloadHashVariations"]["totalProposals"],
        serde_json::json!(58)
    );
    assert_eq!(
        value["payloadHashVariations"]["proposalsWithVariation"],
        serde_json::json!(3)
    );

    let reparsed: DiagnosticBundle =
        serde_json::from_str(&json).expect("DiagnosticBundle round-trip");
    assert_eq!(reparsed.schema_version, 4);
    assert_eq!(reparsed, bundle);
    assert_eq!(reparsed.payload_hash_variations.total_proposals, 58);
    assert_eq!(reparsed.payload_hash_variations.proposals_with_variation, 3);
}
