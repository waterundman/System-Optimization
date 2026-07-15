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
