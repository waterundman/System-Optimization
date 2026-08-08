use serde::{Deserialize, Serialize};

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct SeedDocument {
    pub id: String,
    pub parent_id: Option<String>,
    pub kind: String,
    pub title: String,
    pub order_key: String,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct SeedBlock {
    pub id: String,
    pub document_id: String,
    pub kind: String,
    pub order_key: String,
    pub content_json: String,
    pub plain_text: String,
    pub content_hash: String,
    pub locked: bool,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct ProjectSeed {
    pub project_id: String,
    pub title: String,
    pub language: String,
    pub initial_commit_id: String,
    pub initial_root_hash: String,
    pub main_branch_id: String,
    pub documents: Vec<SeedDocument>,
    pub blocks: Vec<SeedBlock>,
    pub created_at: String,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct ProjectRecord {
    pub id: String,
    pub title: String,
    pub language: String,
    pub schema_version: i64,
    pub head_commit_id: String,
    pub revision: i64,
    pub created_at: String,
    pub updated_at: String,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct BranchRecord {
    pub id: String,
    pub project_id: String,
    pub name: String,
    pub head_commit_id: String,
    pub created_at: String,
    pub updated_at: String,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct DocumentRecord {
    pub id: String,
    pub project_id: String,
    pub parent_id: Option<String>,
    pub kind: String,
    pub title: String,
    pub order_key: String,
    pub revision: i64,
    pub deleted_at: Option<String>,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct DocumentMutation {
    pub document_id: String,
    pub expected_revision: i64,
    pub parent_id: Option<String>,
    pub kind: String,
    pub title: String,
    pub order_key: String,
    pub active: bool,
    pub before_hash: String,
    pub after_hash: String,
    pub operation: String,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct ApplyDocumentBatch {
    pub mutations: Vec<DocumentMutation>,
    pub commit_id: String,
    pub branch_id: String,
    pub expected_head_commit_id: String,
    pub expected_project_revision: i64,
    pub new_root_hash: String,
    pub reason: String,
    pub actor_type: String,
    pub actor_id: Option<String>,
    pub occurred_at: String,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct DocumentBatchReceipt {
    pub commit_id: String,
    pub previous_head_commit_id: String,
    pub project_revision: i64,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct SummaryInvalidationRecord {
    pub project_id: String,
    pub scope_type: String,
    pub scope_id: String,
    pub source_commit_id: String,
    pub reason: String,
    pub invalidation_count: i64,
    pub created_at: String,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct PutSummaryRecord {
    pub project_id: String,
    pub scope_type: String,
    pub scope_id: String,
    pub expected_source_commit_id: String,
    pub source_hash: String,
    pub summary: String,
    pub summary_hash: String,
    pub provider_id: String,
    pub model: String,
    pub generated_at: String,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct SummaryRecord {
    pub project_id: String,
    pub scope_type: String,
    pub scope_id: String,
    pub source_commit_id: String,
    pub source_hash: String,
    pub summary: String,
    pub summary_hash: String,
    pub provider_id: String,
    pub model: String,
    pub revision: i64,
    pub generated_at: String,
    pub updated_at: String,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct StyleSampleRecord {
    pub id: String,
    pub project_id: String,
    pub title: String,
    pub content: String,
    pub content_hash: String,
    pub status: String,
    pub sensitivity: String,
    pub revision: i64,
    pub created_at: String,
    pub updated_at: String,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct CreateStyleSample {
    pub id: String,
    pub project_id: String,
    pub title: String,
    pub content: String,
    pub content_hash: String,
    pub sensitivity: String,
    pub created_at: String,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct SetStyleSampleStatus {
    pub project_id: String,
    pub id: String,
    pub expected_revision: i64,
    pub status: String,
    pub updated_at: String,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct KnowledgeItemRecord {
    pub id: String,
    pub project_id: String,
    pub kind: String,
    pub title: String,
    pub content: String,
    pub content_hash: String,
    pub status: String,
    pub authority: String,
    pub sensitivity: String,
    pub severity: Option<String>,
    pub revision: i64,
    pub created_at: String,
    pub updated_at: String,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct CreateKnowledgeItem {
    pub id: String,
    pub project_id: String,
    pub kind: String,
    pub title: String,
    pub content: String,
    pub content_hash: String,
    pub authority: String,
    pub sensitivity: String,
    pub severity: Option<String>,
    pub created_at: String,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct SetKnowledgeItemStatus {
    pub project_id: String,
    pub id: String,
    pub expected_revision: i64,
    pub status: String,
    pub updated_at: String,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct CreateDocumentWithBlock {
    pub document_id: String,
    pub document_parent_id: Option<String>,
    pub document_kind: String,
    pub document_title: String,
    pub document_order_key: String,
    pub block_id: String,
    pub block_kind: String,
    pub block_order_key: String,
    pub block_content_json: String,
    pub block_plain_text: String,
    pub block_content_hash: String,
    pub block_locked: bool,
    pub commit_id: String,
    pub branch_id: String,
    pub expected_head_commit_id: String,
    pub expected_project_revision: i64,
    pub new_root_hash: String,
    pub actor_type: String,
    pub actor_id: Option<String>,
    pub occurred_at: String,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct CreateDocumentReceipt {
    pub document_id: String,
    pub block_id: String,
    pub commit_id: String,
    pub previous_head_commit_id: String,
    pub project_revision: i64,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct BlockRecord {
    pub id: String,
    pub document_id: String,
    pub kind: String,
    pub order_key: String,
    pub content_json: String,
    pub plain_text: String,
    pub content_hash: String,
    pub revision: i64,
    pub locked: bool,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct ApplyBlockEdit {
    pub edit_id: String,
    pub commit_id: String,
    pub branch_id: String,
    pub expected_head_commit_id: String,
    pub expected_project_revision: i64,
    pub block_id: String,
    pub expected_revision: i64,
    pub expected_hash: String,
    pub new_content_json: String,
    pub new_plain_text: String,
    pub new_content_hash: String,
    pub new_root_hash: String,
    pub reason: String,
    pub actor_type: String,
    pub actor_id: Option<String>,
    pub occurred_at: String,
    pub review_event: Option<AppendReviewEvent>,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct EditReceipt {
    pub edit_id: String,
    pub commit_id: String,
    pub previous_head_commit_id: String,
    pub block_id: String,
    pub new_revision: i64,
    pub new_content_hash: String,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct CommitRecord {
    pub id: String,
    pub project_id: String,
    pub root_hash: String,
    pub reason: String,
    pub actor_type: String,
    pub actor_id: Option<String>,
    pub created_at: String,
    pub parents: Vec<String>,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct CreateSnapshot {
    pub id: String,
    pub project_id: String,
    pub commit_id: String,
    pub root_hash: String,
    pub codec: String,
    pub codec_version: i64,
    pub payload: Vec<u8>,
    pub checksum: String,
    pub created_at: String,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct SnapshotRecord {
    pub id: String,
    pub project_id: String,
    pub commit_id: String,
    pub root_hash: String,
    pub codec: String,
    pub codec_version: i64,
    pub payload: Vec<u8>,
    pub checksum: String,
    pub created_at: String,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct RestoreSnapshot {
    pub snapshot_id: String,
    pub branch_id: String,
    pub new_commit_id: String,
    pub edit_id_prefix: String,
    pub actor_type: String,
    pub actor_id: Option<String>,
    pub occurred_at: String,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct RestoreReceipt {
    pub snapshot_id: String,
    pub commit_id: String,
    pub previous_head_commit_id: String,
    pub restored_root_hash: String,
    pub changed_blocks: usize,
}

#[derive(Debug, Clone, PartialEq)]
pub struct BlockSearchHit {
    pub block_id: String,
    pub document_id: String,
    pub plain_text: String,
    pub rank: f64,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum OperationState {
    Draft,
    Compiling,
    Preflight,
    Queued,
    Streaming,
    Validating,
    Review,
    Accepted,
    Rejected,
    Conflicted,
    Failed,
    Cancelled,
}

impl OperationState {
    pub fn as_str(self) -> &'static str {
        match self {
            Self::Draft => "draft",
            Self::Compiling => "compiling",
            Self::Preflight => "preflight",
            Self::Queued => "queued",
            Self::Streaming => "streaming",
            Self::Validating => "validating",
            Self::Review => "review",
            Self::Accepted => "accepted",
            Self::Rejected => "rejected",
            Self::Conflicted => "conflicted",
            Self::Failed => "failed",
            Self::Cancelled => "cancelled",
        }
    }

    pub(crate) fn parse(value: &str) -> Option<Self> {
        Some(match value {
            "draft" => Self::Draft,
            "compiling" => Self::Compiling,
            "preflight" => Self::Preflight,
            "queued" => Self::Queued,
            "streaming" => Self::Streaming,
            "validating" => Self::Validating,
            "review" => Self::Review,
            "accepted" => Self::Accepted,
            "rejected" => Self::Rejected,
            "conflicted" => Self::Conflicted,
            "failed" => Self::Failed,
            "cancelled" => Self::Cancelled,
            _ => return None,
        })
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum OperationArtifactKind {
    PatchProposal,
    Findings,
}

impl OperationArtifactKind {
    pub fn as_str(self) -> &'static str {
        match self {
            Self::PatchProposal => "patch_proposal",
            Self::Findings => "findings",
        }
    }

    pub(crate) fn parse(value: &str) -> Option<Self> {
        match value {
            "patch_proposal" => Some(Self::PatchProposal),
            "findings" => Some(Self::Findings),
            _ => None,
        }
    }
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct NewContextPacket {
    pub id: String,
    pub operation_intent_id: String,
    pub project_id: String,
    pub base_commit_id: String,
    pub packet_hash: String,
    pub payload_json: String,
    pub created_at: String,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct ContextPacketRecord {
    pub id: String,
    pub operation_intent_id: String,
    pub project_id: String,
    pub base_commit_id: String,
    pub packet_hash: String,
    pub payload_json: String,
    pub created_at: String,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct ModelUsageRecord {
    pub input_tokens: i64,
    pub output_tokens: i64,
    pub total_tokens: i64,
    pub cached_input_tokens: Option<i64>,
    pub reasoning_tokens: Option<i64>,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct OperationFailureRecord {
    pub code: String,
    pub message: String,
    pub retriable: bool,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct NewOperationRun {
    pub id: String,
    pub operation_intent_id: String,
    pub project_id: String,
    pub base_commit_id: String,
    pub provider_id: String,
    pub provider_configuration_id: Option<String>,
    pub provider_endpoint_id: Option<String>,
    pub model: String,
    pub state: OperationState,
    pub response_id: Option<String>,
    pub finish_reason: Option<String>,
    pub usage: Option<ModelUsageRecord>,
    pub failure: Option<OperationFailureRecord>,
    pub started_at: String,
    pub updated_at: String,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct OperationRunRecord {
    pub id: String,
    pub operation_intent_id: String,
    pub project_id: String,
    pub base_commit_id: String,
    pub provider_id: String,
    pub provider_configuration_id: Option<String>,
    pub provider_endpoint_id: Option<String>,
    pub model: String,
    pub state: OperationState,
    pub context_packet_id: Option<String>,
    pub response_id: Option<String>,
    pub finish_reason: Option<String>,
    pub usage: Option<ModelUsageRecord>,
    pub failure: Option<OperationFailureRecord>,
    pub started_at: String,
    pub updated_at: String,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct OperationInsightsRecord {
    pub total_input_tokens: i64,
    pub total_output_tokens: i64,
    pub total_tokens: i64,
    pub accepted_count: i64,
    pub rejected_count: i64,
    pub conflicted_count: i64,
    pub total_runs: i64,
}

#[derive(Debug, Clone, PartialEq, Default, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct RevisionMetrics {
    pub total_proposals_accepted: i64,
    pub proposals_with_rejection: i64,
    pub accepted_after_rejection: i64,
    pub accepted_after_rejection_rate: f64,
}

/// v0.7.0 Stage 2 — payload_hash 变形追踪指标。
///
/// 复用 `patch_review_event.payload_json` JSON 字段存储 `{"payload_hash":"..."}`
/// （来源：`operation_artifact.binding_hash`），统计同一 proposal 下出现
/// 2+ 不同 payload_hash 的变形情况。
///
/// - `total_proposals`: 至少有一个 `patch_review_event` 的 proposal 总数
/// - `proposals_with_variation`: 有 2+ 不同 payload_hash 的 proposal 数
/// - `variations`: 总变形次数（所有 proposal 的不同 payload_hash 数之和减去
///   有 hash 的 proposal 数）
/// - `variation_rate = proposals_with_variation / max(total_proposals, 1)`
///
/// 冷启动期（`total_proposals == 0`）`rate = 0.0` 避免除零。
/// `payload_json` 缺失 `payload_hash` 时 graceful 处理：该 proposal 计入
/// `total_proposals` 但不计入变形统计。
#[derive(Debug, Clone, PartialEq, Default, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct PayloadHashVariations {
    pub total_proposals: i64,
    pub proposals_with_variation: i64,
    pub variations: i64,
    pub variation_rate: f64,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct NewOperationLifecycleEvent {
    pub from_state: OperationState,
    pub to_state: OperationState,
    pub occurred_at: String,
    pub reason: Option<String>,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct OperationLifecycleEventRecord {
    pub run_id: String,
    pub sequence: i64,
    pub from_state: OperationState,
    pub to_state: OperationState,
    pub occurred_at: String,
    pub reason: Option<String>,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct NewOperationAttempt {
    pub sequence: i64,
    pub started_at: String,
    pub finished_at: String,
    pub outcome: String,
    pub response_started: bool,
    pub response_id: Option<String>,
    pub failure_code: Option<String>,
    pub failure_kind: Option<String>,
    pub http_status: Option<i64>,
    pub remote_request_id: Option<String>,
    pub retry_after_ms: Option<i64>,
    pub retriable: Option<bool>,
    pub retry_delay_ms: Option<i64>,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct OperationAttemptRecord {
    pub run_id: String,
    pub sequence: i64,
    pub started_at: String,
    pub finished_at: String,
    pub outcome: String,
    pub response_started: bool,
    pub response_id: Option<String>,
    pub failure_code: Option<String>,
    pub failure_kind: Option<String>,
    pub http_status: Option<i64>,
    pub remote_request_id: Option<String>,
    pub retry_after_ms: Option<i64>,
    pub retriable: Option<bool>,
    pub retry_delay_ms: Option<i64>,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct NewOperationArtifact {
    pub id: String,
    pub kind: OperationArtifactKind,
    pub binding_hash: String,
    pub payload_json: String,
    pub created_at: String,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct OperationArtifactRecord {
    pub id: String,
    pub run_id: String,
    pub kind: OperationArtifactKind,
    pub binding_hash: String,
    pub payload_json: String,
    pub created_at: String,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct PersistOperationBundle {
    pub run: NewOperationRun,
    pub context_packet: Option<NewContextPacket>,
    pub lifecycle_events: Vec<NewOperationLifecycleEvent>,
    pub attempts: Vec<NewOperationAttempt>,
    pub artifact: Option<NewOperationArtifact>,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum ReviewSessionStatus {
    Review,
    Ready,
    Applied,
    Rejected,
    Conflicted,
}

impl ReviewSessionStatus {
    pub fn as_str(self) -> &'static str {
        match self {
            Self::Review => "review",
            Self::Ready => "ready",
            Self::Applied => "applied",
            Self::Rejected => "rejected",
            Self::Conflicted => "conflicted",
        }
    }

    pub(crate) fn parse(value: &str) -> Option<Self> {
        match value {
            "review" => Some(Self::Review),
            "ready" => Some(Self::Ready),
            "applied" => Some(Self::Applied),
            "rejected" => Some(Self::Rejected),
            "conflicted" => Some(Self::Conflicted),
            _ => None,
        }
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum ReviewEventKind {
    Decision,
    Apply,
    Conflict,
    Rebase,
    Reject,
}

impl ReviewEventKind {
    pub fn as_str(self) -> &'static str {
        match self {
            Self::Decision => "decision",
            Self::Apply => "apply",
            Self::Conflict => "conflict",
            Self::Rebase => "rebase",
            Self::Reject => "reject",
        }
    }

    pub(crate) fn parse(value: &str) -> Option<Self> {
        match value {
            "decision" => Some(Self::Decision),
            "apply" => Some(Self::Apply),
            "conflict" => Some(Self::Conflict),
            "rebase" => Some(Self::Rebase),
            "reject" => Some(Self::Reject),
            _ => None,
        }
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum ReviewDecision {
    Accepted,
    Rejected,
}

impl ReviewDecision {
    pub fn as_str(self) -> &'static str {
        match self {
            Self::Accepted => "accepted",
            Self::Rejected => "rejected",
        }
    }

    pub(crate) fn parse(value: &str) -> Option<Self> {
        match value {
            "accepted" => Some(Self::Accepted),
            "rejected" => Some(Self::Rejected),
            _ => None,
        }
    }
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct AppendReviewEvent {
    pub id: String,
    pub proposal_id: String,
    pub expected_revision: i64,
    pub expected_status: ReviewSessionStatus,
    pub kind: ReviewEventKind,
    pub next_status: ReviewSessionStatus,
    pub hunk_id: Option<String>,
    pub decision: Option<ReviewDecision>,
    pub payload_json: Option<String>,
    pub occurred_at: String,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct ReviewSessionRecord {
    pub proposal_id: String,
    pub run_id: String,
    pub revision: i64,
    pub status: ReviewSessionStatus,
    pub updated_at: String,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct ReviewEventRecord {
    pub id: String,
    pub proposal_id: String,
    pub sequence: i64,
    pub base_revision: i64,
    pub new_revision: i64,
    pub kind: ReviewEventKind,
    pub previous_status: ReviewSessionStatus,
    pub next_status: ReviewSessionStatus,
    pub hunk_id: Option<String>,
    pub decision: Option<ReviewDecision>,
    pub payload_json: Option<String>,
    pub occurred_at: String,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct ReviewCandidateBranchRecord {
    pub proposal_id: String,
    pub branch_id: String,
    pub branch_name: String,
    pub commit_id: String,
    pub snapshot_id: String,
    pub created_at: String,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct ReviewCandidateSummaryRecord {
    pub proposal_id: String,
    pub run_id: String,
    pub project_id: String,
    pub operation_intent_id: String,
    pub provider_id: String,
    pub model: String,
    pub base_commit_id: String,
    pub target_document_id: String,
    pub target_block_id: String,
    pub hunk_count: i64,
    pub summary: Option<String>,
    pub revision: i64,
    pub status: ReviewSessionStatus,
    pub created_at: String,
    pub updated_at: String,
    pub candidate_branch: Option<ReviewCandidateBranchRecord>,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct CreateReviewCandidateBranch {
    pub proposal_id: String,
    pub expected_review_revision: i64,
    pub project_id: String,
    pub expected_project_head_commit_id: String,
    pub branch_id: String,
    pub branch_name: String,
    pub commit_id: String,
    pub snapshot_id: String,
    pub target_document_id: String,
    pub target_block_id: String,
    pub expected_block_revision: i64,
    pub expected_block_hash: String,
    pub new_content_json: String,
    pub new_plain_text: String,
    pub new_content_hash: String,
    pub new_root_hash: String,
    pub occurred_at: String,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "lowercase")]
pub enum BlockDiffKind {
    Unchanged,
    Added,
    Removed,
    Modified,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "lowercase")]
pub enum TextDiffOp {
    Equal(String),
    Delete(String),
    Insert(String),
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct DiffSummary {
    pub added_count: i64,
    pub removed_count: i64,
    pub modified_count: i64,
    pub unchanged_count: i64,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct BlockDiffEntry {
    pub block_id_a: Option<String>,
    pub block_id_b: Option<String>,
    pub kind: BlockDiffKind,
    pub text_diff: Option<Vec<TextDiffOp>>,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct DocumentDiffResult {
    pub document_id_a: String,
    pub document_id_b: String,
    pub blocks: Vec<BlockDiffEntry>,
    pub summary: DiffSummary,
}

/// v0.8.0 Stage 1 (FR-11) — project-level backup manifest.
///
/// Serialized as `manifest.json` at the root of every `.optimizer-backup`
/// zip archive. `#[serde(default)]` on every field so future schema
/// additions (new optional metadata) do not break deserialization of
/// older backups — the contract requires T14 (missing fields must not
/// error).
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct BackupManifest {
    #[serde(default)]
    pub schema_version: i64,
    #[serde(default)]
    pub source_project_id: String,
    #[serde(default)]
    pub source_path: String,
    #[serde(default)]
    pub generated_at: String,
    #[serde(default)]
    pub included_items: Vec<String>,
    #[serde(default)]
    pub optimizer_version: String,
    #[serde(default)]
    pub schema_db_version: i64,
}

#[cfg(test)]
mod backup_manifest_tests {
    use super::BackupManifest;

    /// T13: manifest.json can be deserialized into BackupManifest.
    #[test]
    fn t13_manifest_json_deserializes_into_backup_manifest() {
        let json = r#"{
            "schemaVersion": 1,
            "sourceProjectId": "project-abc-123",
            "sourcePath": "W:\\writing\\novel.optimizer",
            "generatedAt": "2026-08-03T12:00:00.000Z",
            "includedItems": ["project.sqlite3", "manifest.json", "endpoints.json"],
            "optimizerVersion": "0.8.0",
            "schemaDbVersion": 11
        }"#;
        let manifest: BackupManifest = serde_json::from_str(json)
            .expect("manifest.json must deserialize into BackupManifest");
        assert_eq!(manifest.schema_version, 1);
        assert_eq!(manifest.source_project_id, "project-abc-123");
        assert_eq!(manifest.source_path, "W:\\writing\\novel.optimizer");
        assert_eq!(manifest.generated_at, "2026-08-03T12:00:00.000Z");
        assert_eq!(
            manifest.included_items,
            vec!["project.sqlite3", "manifest.json", "endpoints.json"]
        );
        assert_eq!(manifest.optimizer_version, "0.8.0");
        assert_eq!(manifest.schema_db_version, 11);
    }

    /// T14: BackupManifest #[serde(default)] tolerates missing fields.
    #[test]
    fn t14_backup_manifest_serde_default_tolerates_missing_fields() {
        let json = r#"{"schemaVersion": 1}"#;
        let manifest: BackupManifest = serde_json::from_str(json)
            .expect("missing fields must not cause an error due to #[serde(default)]");
        assert_eq!(manifest.schema_version, 1);
        assert_eq!(manifest.source_project_id, "");
        assert_eq!(manifest.source_path, "");
        assert_eq!(manifest.generated_at, "");
        assert!(manifest.included_items.is_empty());
        assert_eq!(manifest.optimizer_version, "");
        assert_eq!(manifest.schema_db_version, 0);

        let empty_json = r#"{}"#;
        let empty_manifest: BackupManifest = serde_json::from_str(empty_json)
            .expect("completely empty object must not error");
        assert_eq!(empty_manifest.schema_version, 0);
        assert!(empty_manifest.included_items.is_empty());
    }

    /// T13 bonus: round-trip serialization preserves all fields.
    #[test]
    fn t13_backup_manifest_round_trips_through_serde() {
        let original = BackupManifest {
            schema_version: 1,
            source_project_id: "project-round-trip".into(),
            source_path: "/tmp/test.optimizer".into(),
            generated_at: "2026-08-03T10:00:00.000Z".into(),
            included_items: vec!["project.sqlite3".into(), "manifest.json".into()],
            optimizer_version: "0.8.0".into(),
            schema_db_version: 11,
        };
        let json = serde_json::to_string(&original).expect("serialize must succeed");
        let round_tripped: BackupManifest =
            serde_json::from_str(&json).expect("deserialize must succeed");
        assert_eq!(original, round_tripped);
    }
}
