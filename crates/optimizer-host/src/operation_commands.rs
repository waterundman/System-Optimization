use std::fmt;

use optimizer_store::{
    AppendReviewEvent, ContextPacketRecord, ModelUsageRecord, NewContextPacket,
    NewOperationArtifact, NewOperationLifecycleEvent, NewOperationRun, OperationArtifactKind,
    OperationArtifactRecord, OperationFailureRecord, OperationLifecycleEventRecord,
    OperationRunRecord, OperationState, OptimizerStore, PersistOperationBundle, ReviewDecision,
    ReviewEventKind, ReviewEventRecord, ReviewSessionRecord, ReviewSessionStatus, StoreError,
};
use serde::{Deserialize, Serialize};
use serde_json::Value;

#[derive(Debug)]
pub enum OperationCommandError {
    Json(serde_json::Error),
    UnsupportedSchema { command: &'static str, actual: u32 },
    SensitiveField { path: String },
    Store(StoreError),
}

impl fmt::Display for OperationCommandError {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            Self::Json(error) => write!(formatter, "invalid host command JSON: {error}"),
            Self::UnsupportedSchema { command, actual } => {
                write!(formatter, "unsupported {command} schema version {actual}")
            }
            Self::SensitiveField { path } => {
                write!(
                    formatter,
                    "sensitive credential field is forbidden at {path}"
                )
            }
            Self::Store(error) => write!(formatter, "operation store command failed: {error}"),
        }
    }
}

impl std::error::Error for OperationCommandError {}

impl From<serde_json::Error> for OperationCommandError {
    fn from(value: serde_json::Error) -> Self {
        Self::Json(value)
    }
}

impl From<StoreError> for OperationCommandError {
    fn from(value: StoreError) -> Self {
        Self::Store(value)
    }
}

pub struct OperationCommandHost {
    store: OptimizerStore,
}

impl OperationCommandHost {
    pub fn new(store: OptimizerStore) -> Self {
        Self { store }
    }

    pub fn persist_operation_bundle_json(
        &mut self,
        input: &str,
    ) -> Result<PersistOperationResponse, OperationCommandError> {
        let command: PersistOperationBundleDto = serde_json::from_str(input)?;
        if command.schema_version != 1 {
            return Err(OperationCommandError::UnsupportedSchema {
                command: "operation persistence bundle",
                actual: command.schema_version,
            });
        }
        command.ensure_safe()?;
        let artifact_id = command
            .artifact
            .as_ref()
            .map(|artifact| artifact.id.clone());
        let record = self
            .store
            .persist_operation_bundle(&command.try_into_store()?)?;
        Ok(PersistOperationResponse {
            schema_version: 1,
            run_id: record.id,
            state: record.state.as_str().into(),
            context_packet_id: record.context_packet_id,
            artifact_id,
        })
    }

    pub fn append_review_event_json(
        &mut self,
        input: &str,
    ) -> Result<PersistReviewResponse, OperationCommandError> {
        let command: PersistReviewEventDto = serde_json::from_str(input)?;
        if command.schema_version != 1 {
            return Err(OperationCommandError::UnsupportedSchema {
                command: "review event",
                actual: command.schema_version,
            });
        }
        command.ensure_safe()?;
        let session = self.store.append_review_event(&command.into_store()?)?;
        let run = self.store.get_operation_run(&session.run_id)?;
        Ok(PersistReviewResponse {
            schema_version: 1,
            proposal_id: session.proposal_id,
            revision: session.revision,
            status: session.status.as_str().into(),
            run_id: run.id,
            operation_state: run.state.as_str().into(),
        })
    }

    pub fn get_operation_audit(
        &self,
        run_id: &str,
    ) -> Result<OperationAuditResponse, OperationCommandError> {
        let run = self.store.get_operation_run(run_id)?;
        let context_packet = run
            .context_packet_id
            .as_deref()
            .map(|id| self.store.get_context_packet(id))
            .transpose()?
            .map(ContextPacketAudit::try_from)
            .transpose()?;
        let lifecycle_events = self
            .store
            .list_operation_lifecycle_events(run_id)?
            .into_iter()
            .map(LifecycleEventAudit::from)
            .collect();
        let artifact = match self.store.get_operation_artifact(run_id) {
            Ok(record) => Some(ArtifactAudit::try_from(record)?),
            Err(StoreError::NotFound { .. }) => None,
            Err(error) => return Err(error.into()),
        };
        let review = if let Some(artifact) = &artifact {
            if artifact.kind == "patch_proposal" {
                let session = self.store.get_review_session(&artifact.id)?;
                let events = self.store.list_review_events(&artifact.id)?;
                Some(ReviewAudit::try_from_records(session, events)?)
            } else {
                None
            }
        } else {
            None
        };
        Ok(OperationAuditResponse {
            schema_version: 1,
            run: RunAudit::from(run),
            context_packet,
            lifecycle_events,
            artifact,
            review,
        })
    }

    pub fn into_store(self) -> OptimizerStore {
        self.store
    }
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct PersistOperationResponse {
    pub schema_version: u32,
    pub run_id: String,
    pub state: String,
    pub context_packet_id: Option<String>,
    pub artifact_id: Option<String>,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct PersistReviewResponse {
    pub schema_version: u32,
    pub proposal_id: String,
    pub revision: i64,
    pub status: String,
    pub run_id: String,
    pub operation_state: String,
}

#[derive(Debug, Clone, PartialEq, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct OperationAuditResponse {
    pub schema_version: u32,
    pub run: RunAudit,
    pub context_packet: Option<ContextPacketAudit>,
    pub lifecycle_events: Vec<LifecycleEventAudit>,
    pub artifact: Option<ArtifactAudit>,
    pub review: Option<ReviewAudit>,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct RunAudit {
    pub id: String,
    pub operation_intent_id: String,
    pub project_id: String,
    pub base_commit_id: String,
    pub provider_id: String,
    pub model: String,
    pub state: String,
    pub context_packet_id: Option<String>,
    pub response_id: Option<String>,
    pub finish_reason: Option<String>,
    pub usage: Option<ModelUsageAudit>,
    pub failure: Option<FailureAudit>,
    pub started_at: String,
    pub updated_at: String,
}

impl From<OperationRunRecord> for RunAudit {
    fn from(record: OperationRunRecord) -> Self {
        Self {
            id: record.id,
            operation_intent_id: record.operation_intent_id,
            project_id: record.project_id,
            base_commit_id: record.base_commit_id,
            provider_id: record.provider_id,
            model: record.model,
            state: record.state.as_str().into(),
            context_packet_id: record.context_packet_id,
            response_id: record.response_id,
            finish_reason: record.finish_reason,
            usage: record.usage.map(ModelUsageAudit::from),
            failure: record.failure.map(FailureAudit::from),
            started_at: record.started_at,
            updated_at: record.updated_at,
        }
    }
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct ModelUsageAudit {
    pub input_tokens: i64,
    pub output_tokens: i64,
    pub total_tokens: i64,
    pub cached_input_tokens: Option<i64>,
    pub reasoning_tokens: Option<i64>,
}

impl From<ModelUsageRecord> for ModelUsageAudit {
    fn from(value: ModelUsageRecord) -> Self {
        Self {
            input_tokens: value.input_tokens,
            output_tokens: value.output_tokens,
            total_tokens: value.total_tokens,
            cached_input_tokens: value.cached_input_tokens,
            reasoning_tokens: value.reasoning_tokens,
        }
    }
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct FailureAudit {
    pub code: String,
    pub message: String,
    pub retriable: bool,
}

impl From<OperationFailureRecord> for FailureAudit {
    fn from(value: OperationFailureRecord) -> Self {
        Self {
            code: value.code,
            message: value.message,
            retriable: value.retriable,
        }
    }
}

#[derive(Debug, Clone, PartialEq, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct ContextPacketAudit {
    pub id: String,
    pub operation_intent_id: String,
    pub project_id: String,
    pub base_commit_id: String,
    pub packet_hash: String,
    pub payload: Value,
    pub created_at: String,
}

impl TryFrom<ContextPacketRecord> for ContextPacketAudit {
    type Error = OperationCommandError;

    fn try_from(value: ContextPacketRecord) -> Result<Self, Self::Error> {
        Ok(Self {
            id: value.id,
            operation_intent_id: value.operation_intent_id,
            project_id: value.project_id,
            base_commit_id: value.base_commit_id,
            packet_hash: value.packet_hash,
            payload: serde_json::from_str(&value.payload_json)?,
            created_at: value.created_at,
        })
    }
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct LifecycleEventAudit {
    pub sequence: i64,
    pub from_state: String,
    pub to_state: String,
    pub occurred_at: String,
    pub reason: Option<String>,
}

impl From<OperationLifecycleEventRecord> for LifecycleEventAudit {
    fn from(value: OperationLifecycleEventRecord) -> Self {
        Self {
            sequence: value.sequence,
            from_state: value.from_state.as_str().into(),
            to_state: value.to_state.as_str().into(),
            occurred_at: value.occurred_at,
            reason: value.reason,
        }
    }
}

#[derive(Debug, Clone, PartialEq, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct ArtifactAudit {
    pub id: String,
    pub kind: String,
    pub binding_hash: String,
    pub payload: Value,
    pub created_at: String,
}

impl TryFrom<OperationArtifactRecord> for ArtifactAudit {
    type Error = OperationCommandError;

    fn try_from(value: OperationArtifactRecord) -> Result<Self, Self::Error> {
        Ok(Self {
            id: value.id,
            kind: value.kind.as_str().into(),
            binding_hash: value.binding_hash,
            payload: serde_json::from_str(&value.payload_json)?,
            created_at: value.created_at,
        })
    }
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct ReviewAudit {
    pub proposal_id: String,
    pub revision: i64,
    pub status: String,
    pub updated_at: String,
    pub events: Vec<ReviewEventAudit>,
}

impl ReviewAudit {
    fn try_from_records(
        session: ReviewSessionRecord,
        events: Vec<ReviewEventRecord>,
    ) -> Result<Self, OperationCommandError> {
        Ok(Self {
            proposal_id: session.proposal_id,
            revision: session.revision,
            status: session.status.as_str().into(),
            updated_at: session.updated_at,
            events: events
                .into_iter()
                .map(ReviewEventAudit::try_from)
                .collect::<Result<_, _>>()?,
        })
    }
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct ReviewEventAudit {
    pub id: String,
    pub sequence: i64,
    pub base_revision: i64,
    pub new_revision: i64,
    pub kind: String,
    pub previous_status: String,
    pub next_status: String,
    pub hunk_id: Option<String>,
    pub decision: Option<String>,
    pub payload: Option<Value>,
    pub occurred_at: String,
}

impl TryFrom<ReviewEventRecord> for ReviewEventAudit {
    type Error = OperationCommandError;

    fn try_from(value: ReviewEventRecord) -> Result<Self, Self::Error> {
        Ok(Self {
            id: value.id,
            sequence: value.sequence,
            base_revision: value.base_revision,
            new_revision: value.new_revision,
            kind: value.kind.as_str().into(),
            previous_status: value.previous_status.as_str().into(),
            next_status: value.next_status.as_str().into(),
            hunk_id: value.hunk_id,
            decision: value.decision.map(|decision| decision.as_str().into()),
            payload: value
                .payload_json
                .map(|payload| serde_json::from_str(&payload))
                .transpose()?,
            occurred_at: value.occurred_at,
        })
    }
}

#[derive(Debug, Deserialize)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
struct PersistOperationBundleDto {
    schema_version: u32,
    run: RunDto,
    context_packet: Option<ContextPacketDto>,
    lifecycle_events: Vec<LifecycleEventDto>,
    artifact: Option<ArtifactDto>,
}

impl PersistOperationBundleDto {
    fn ensure_safe(&self) -> Result<(), OperationCommandError> {
        if let Some(packet) = &self.context_packet {
            reject_sensitive_fields(&packet.payload, "contextPacket.payload")?;
        }
        if let Some(artifact) = &self.artifact {
            reject_sensitive_fields(&artifact.payload, "artifact.payload")?;
        }
        Ok(())
    }

    fn try_into_store(self) -> Result<PersistOperationBundle, OperationCommandError> {
        Ok(PersistOperationBundle {
            run: self.run.into_store(),
            context_packet: self
                .context_packet
                .map(ContextPacketDto::try_into_store)
                .transpose()?,
            lifecycle_events: self
                .lifecycle_events
                .into_iter()
                .map(LifecycleEventDto::into_store)
                .collect(),
            artifact: self.artifact.map(ArtifactDto::try_into_store).transpose()?,
        })
    }
}

#[derive(Debug, Deserialize)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
struct RunDto {
    id: String,
    operation_intent_id: String,
    project_id: String,
    base_commit_id: String,
    provider_id: String,
    model: String,
    state: OperationStateDto,
    response_id: Option<String>,
    finish_reason: Option<String>,
    usage: Option<ModelUsageDto>,
    failure: Option<FailureDto>,
    started_at: String,
    updated_at: String,
}

impl RunDto {
    fn into_store(self) -> NewOperationRun {
        NewOperationRun {
            id: self.id,
            operation_intent_id: self.operation_intent_id,
            project_id: self.project_id,
            base_commit_id: self.base_commit_id,
            provider_id: self.provider_id,
            model: self.model,
            state: self.state.into(),
            response_id: self.response_id,
            finish_reason: self.finish_reason,
            usage: self.usage.map(ModelUsageDto::into_store),
            failure: self.failure.map(FailureDto::into_store),
            started_at: self.started_at,
            updated_at: self.updated_at,
        }
    }
}

#[derive(Debug, Deserialize)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
struct ModelUsageDto {
    input_tokens: i64,
    output_tokens: i64,
    total_tokens: i64,
    cached_input_tokens: Option<i64>,
    reasoning_tokens: Option<i64>,
}

impl ModelUsageDto {
    fn into_store(self) -> ModelUsageRecord {
        ModelUsageRecord {
            input_tokens: self.input_tokens,
            output_tokens: self.output_tokens,
            total_tokens: self.total_tokens,
            cached_input_tokens: self.cached_input_tokens,
            reasoning_tokens: self.reasoning_tokens,
        }
    }
}

#[derive(Debug, Deserialize)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
struct FailureDto {
    code: String,
    message: String,
    retriable: bool,
}

impl FailureDto {
    fn into_store(self) -> OperationFailureRecord {
        OperationFailureRecord {
            code: self.code,
            message: self.message,
            retriable: self.retriable,
        }
    }
}

#[derive(Debug, Deserialize)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
struct ContextPacketDto {
    id: String,
    operation_intent_id: String,
    project_id: String,
    base_commit_id: String,
    packet_hash: String,
    payload: Value,
    created_at: String,
}

impl ContextPacketDto {
    fn try_into_store(self) -> Result<NewContextPacket, OperationCommandError> {
        Ok(NewContextPacket {
            id: self.id,
            operation_intent_id: self.operation_intent_id,
            project_id: self.project_id,
            base_commit_id: self.base_commit_id,
            packet_hash: self.packet_hash,
            payload_json: serde_json::to_string(&self.payload)?,
            created_at: self.created_at,
        })
    }
}

#[derive(Debug, Deserialize)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
struct LifecycleEventDto {
    from_state: OperationStateDto,
    to_state: OperationStateDto,
    occurred_at: String,
    reason: Option<String>,
}

impl LifecycleEventDto {
    fn into_store(self) -> NewOperationLifecycleEvent {
        NewOperationLifecycleEvent {
            from_state: self.from_state.into(),
            to_state: self.to_state.into(),
            occurred_at: self.occurred_at,
            reason: self.reason,
        }
    }
}

#[derive(Debug, Deserialize)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
struct ArtifactDto {
    id: String,
    kind: ArtifactKindDto,
    binding_hash: String,
    payload: Value,
    created_at: String,
}

impl ArtifactDto {
    fn try_into_store(self) -> Result<NewOperationArtifact, OperationCommandError> {
        Ok(NewOperationArtifact {
            id: self.id,
            kind: self.kind.into(),
            binding_hash: self.binding_hash,
            payload_json: serde_json::to_string(&self.payload)?,
            created_at: self.created_at,
        })
    }
}

#[derive(Debug, Deserialize)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
struct PersistReviewEventDto {
    schema_version: u32,
    id: String,
    proposal_id: String,
    expected_revision: i64,
    expected_status: ReviewStatusDto,
    kind: ReviewEventKindDto,
    next_status: ReviewStatusDto,
    hunk_id: Option<String>,
    decision: Option<ReviewDecisionDto>,
    payload: Option<Value>,
    occurred_at: String,
}

impl PersistReviewEventDto {
    fn ensure_safe(&self) -> Result<(), OperationCommandError> {
        if let Some(payload) = &self.payload {
            reject_sensitive_fields(payload, "payload")?;
        }
        Ok(())
    }

    fn into_store(self) -> Result<AppendReviewEvent, OperationCommandError> {
        Ok(AppendReviewEvent {
            id: self.id,
            proposal_id: self.proposal_id,
            expected_revision: self.expected_revision,
            expected_status: self.expected_status.into(),
            kind: self.kind.into(),
            next_status: self.next_status.into(),
            hunk_id: self.hunk_id,
            decision: self.decision.map(Into::into),
            payload_json: self
                .payload
                .map(|value| serde_json::to_string(&value))
                .transpose()?,
            occurred_at: self.occurred_at,
        })
    }
}

#[derive(Debug, Clone, Copy, Deserialize)]
#[serde(rename_all = "snake_case")]
enum OperationStateDto {
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

impl From<OperationStateDto> for OperationState {
    fn from(value: OperationStateDto) -> Self {
        match value {
            OperationStateDto::Draft => Self::Draft,
            OperationStateDto::Compiling => Self::Compiling,
            OperationStateDto::Preflight => Self::Preflight,
            OperationStateDto::Queued => Self::Queued,
            OperationStateDto::Streaming => Self::Streaming,
            OperationStateDto::Validating => Self::Validating,
            OperationStateDto::Review => Self::Review,
            OperationStateDto::Accepted => Self::Accepted,
            OperationStateDto::Rejected => Self::Rejected,
            OperationStateDto::Conflicted => Self::Conflicted,
            OperationStateDto::Failed => Self::Failed,
            OperationStateDto::Cancelled => Self::Cancelled,
        }
    }
}

#[derive(Debug, Clone, Copy, Deserialize)]
#[serde(rename_all = "snake_case")]
enum ArtifactKindDto {
    PatchProposal,
    Findings,
}

impl From<ArtifactKindDto> for OperationArtifactKind {
    fn from(value: ArtifactKindDto) -> Self {
        match value {
            ArtifactKindDto::PatchProposal => Self::PatchProposal,
            ArtifactKindDto::Findings => Self::Findings,
        }
    }
}

#[derive(Debug, Clone, Copy, Deserialize)]
#[serde(rename_all = "snake_case")]
enum ReviewStatusDto {
    Review,
    Ready,
    Applied,
    Rejected,
    Conflicted,
}

impl From<ReviewStatusDto> for ReviewSessionStatus {
    fn from(value: ReviewStatusDto) -> Self {
        match value {
            ReviewStatusDto::Review => Self::Review,
            ReviewStatusDto::Ready => Self::Ready,
            ReviewStatusDto::Applied => Self::Applied,
            ReviewStatusDto::Rejected => Self::Rejected,
            ReviewStatusDto::Conflicted => Self::Conflicted,
        }
    }
}

#[derive(Debug, Clone, Copy, Deserialize)]
#[serde(rename_all = "snake_case")]
enum ReviewEventKindDto {
    Decision,
    Apply,
    Conflict,
    Rebase,
    Reject,
}

impl From<ReviewEventKindDto> for ReviewEventKind {
    fn from(value: ReviewEventKindDto) -> Self {
        match value {
            ReviewEventKindDto::Decision => Self::Decision,
            ReviewEventKindDto::Apply => Self::Apply,
            ReviewEventKindDto::Conflict => Self::Conflict,
            ReviewEventKindDto::Rebase => Self::Rebase,
            ReviewEventKindDto::Reject => Self::Reject,
        }
    }
}

#[derive(Debug, Clone, Copy, Deserialize)]
#[serde(rename_all = "snake_case")]
enum ReviewDecisionDto {
    Accepted,
    Rejected,
}

impl From<ReviewDecisionDto> for ReviewDecision {
    fn from(value: ReviewDecisionDto) -> Self {
        match value {
            ReviewDecisionDto::Accepted => Self::Accepted,
            ReviewDecisionDto::Rejected => Self::Rejected,
        }
    }
}

fn reject_sensitive_fields(value: &Value, path: &str) -> Result<(), OperationCommandError> {
    match value {
        Value::Object(object) => {
            for (key, nested) in object {
                let normalized = key
                    .chars()
                    .filter(|character| character.is_ascii_alphanumeric())
                    .flat_map(char::to_lowercase)
                    .collect::<String>();
                if matches!(
                    normalized.as_str(),
                    "apikey"
                        | "authorization"
                        | "proxyauthorization"
                        | "credential"
                        | "credentialvalue"
                        | "secret"
                        | "secretvalue"
                        | "headers"
                        | "rawheaders"
                ) {
                    return Err(OperationCommandError::SensitiveField {
                        path: format!("{path}.{key}"),
                    });
                }
                reject_sensitive_fields(nested, &format!("{path}.{key}"))?;
            }
        }
        Value::Array(values) => {
            for (index, nested) in values.iter().enumerate() {
                reject_sensitive_fields(nested, &format!("{path}[{index}]"))?;
            }
        }
        _ => {}
    }
    Ok(())
}

#[cfg(test)]
mod tests {
    use std::fs;
    use std::path::Path;

    use optimizer_store::{ProjectSeed, SeedBlock, SeedDocument};
    use serde_json::json;

    use super::*;

    fn seeded_host() -> OperationCommandHost {
        let mut store = OptimizerStore::open_in_memory().unwrap();
        store
            .initialize_project(&ProjectSeed {
                project_id: "project-1".into(),
                title: "Host command test".into(),
                language: "zh-CN".into(),
                initial_commit_id: "commit-initial".into(),
                initial_root_hash: "sha256:root-initial".into(),
                main_branch_id: "branch-main".into(),
                documents: vec![SeedDocument {
                    id: "document-1".into(),
                    parent_id: None,
                    kind: "chapter".into(),
                    title: "Chapter 1".into(),
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
            })
            .unwrap();
        OperationCommandHost::new(store)
    }

    fn shared_fixture() -> String {
        let path = Path::new(env!("CARGO_MANIFEST_DIR"))
            .join("../../packages/protocol/fixtures/operation-persistence-bundle.v1.json");
        fs::read_to_string(path).unwrap()
    }

    #[test]
    fn persists_the_shared_typescript_fixture_and_reads_a_complete_audit() {
        let mut host = seeded_host();
        let response = host
            .persist_operation_bundle_json(&shared_fixture())
            .unwrap();
        assert_eq!(response.run_id, "run-fixture-1");
        assert_eq!(response.artifact_id.as_deref(), Some("proposal-fixture-1"));

        let audit = host.get_operation_audit("run-fixture-1").unwrap();
        assert_eq!(audit.run.provider_id, "deepseek");
        assert_eq!(audit.run.usage.unwrap().cached_input_tokens, Some(12));
        assert_eq!(audit.lifecycle_events.len(), 6);
        assert_eq!(audit.context_packet.unwrap().payload["schemaVersion"], 1);
        assert_eq!(audit.artifact.unwrap().kind, "patch_proposal");
        let review = audit.review.unwrap();
        assert_eq!(review.revision, 0);
        assert!(review.events.is_empty());
    }

    #[test]
    fn rejects_unknown_nested_fields_without_partial_persistence() {
        let mut host = seeded_host();
        let mut fixture: Value = serde_json::from_str(&shared_fixture()).unwrap();
        fixture["run"]["apiKey"] = json!("must-never-cross-the-host-boundary");

        assert!(matches!(
            host.persist_operation_bundle_json(&fixture.to_string()),
            Err(OperationCommandError::Json(_))
        ));
        assert!(matches!(
            host.get_operation_audit("run-fixture-1"),
            Err(OperationCommandError::Store(StoreError::NotFound { .. }))
        ));
    }

    #[test]
    fn rejects_credential_fields_hidden_inside_opaque_payloads() {
        let mut host = seeded_host();
        let mut fixture: Value = serde_json::from_str(&shared_fixture()).unwrap();
        fixture["contextPacket"]["payload"]["transport"] = json!({
            "apiKey": "must-never-be-persisted"
        });

        assert!(matches!(
            host.persist_operation_bundle_json(&fixture.to_string()),
            Err(OperationCommandError::SensitiveField { path }) if path.ends_with("transport.apiKey")
        ));
        assert!(matches!(
            host.get_operation_audit("run-fixture-1"),
            Err(OperationCommandError::Store(StoreError::NotFound { .. }))
        ));
    }

    #[test]
    fn rejects_unsupported_versions_before_persistence() {
        let mut host = seeded_host();
        let mut fixture: Value = serde_json::from_str(&shared_fixture()).unwrap();
        fixture["schemaVersion"] = json!(2);

        assert!(matches!(
            host.persist_operation_bundle_json(&fixture.to_string()),
            Err(OperationCommandError::UnsupportedSchema { actual: 2, .. })
        ));
        assert!(matches!(
            host.get_operation_audit("run-fixture-1"),
            Err(OperationCommandError::Store(StoreError::NotFound { .. }))
        ));
    }

    #[test]
    fn persists_a_failed_operation_without_context_or_artifact() {
        let mut host = seeded_host();
        let failed = json!({
            "schemaVersion": 1,
            "run": {
                "id": "run-failed-host-1",
                "operationIntentId": "intent-failed-host-1",
                "projectId": "project-1",
                "baseCommitId": "commit-initial",
                "providerId": "qwen",
                "model": "qwen-plus",
                "state": "failed",
                "failure": {
                    "code": "PROVIDER_FAILED",
                    "message": "Provider request failed",
                    "retriable": true
                },
                "startedAt": "2026-07-15T00:00:00.000Z",
                "updatedAt": "2026-07-15T00:00:10.000Z"
            },
            "lifecycleEvents": [
                {
                    "fromState": "draft",
                    "toState": "compiling",
                    "occurredAt": "2026-07-15T00:00:00.000Z"
                },
                {
                    "fromState": "compiling",
                    "toState": "failed",
                    "occurredAt": "2026-07-15T00:00:10.000Z",
                    "reason": "PROVIDER_FAILED"
                }
            ]
        });

        let response = host
            .persist_operation_bundle_json(&failed.to_string())
            .unwrap();
        assert_eq!(response.state, "failed");
        assert_eq!(response.context_packet_id, None);
        assert_eq!(response.artifact_id, None);

        let audit = host.get_operation_audit("run-failed-host-1").unwrap();
        assert!(audit.run.failure.unwrap().retriable);
        assert!(audit.context_packet.is_none());
        assert!(audit.artifact.is_none());
        assert!(audit.review.is_none());
    }

    #[test]
    fn persists_review_commands_and_exposes_the_resulting_operation_state() {
        let mut host = seeded_host();
        host.persist_operation_bundle_json(&shared_fixture())
            .unwrap();
        let decision = json!({
            "schemaVersion": 1,
            "id": "review-event-1",
            "proposalId": "proposal-fixture-1",
            "expectedRevision": 0,
            "expectedStatus": "review",
            "kind": "decision",
            "nextStatus": "ready",
            "hunkId": "hunk-1",
            "decision": "accepted",
            "payload": { "source": "inline-review" },
            "occurredAt": "2026-07-15T00:02:00.000Z"
        });
        let response = host
            .append_review_event_json(&decision.to_string())
            .unwrap();
        assert_eq!(response.revision, 1);
        assert_eq!(response.status, "ready");
        assert_eq!(response.operation_state, "review");

        let apply = json!({
            "schemaVersion": 1,
            "id": "review-event-2",
            "proposalId": "proposal-fixture-1",
            "expectedRevision": 1,
            "expectedStatus": "ready",
            "kind": "apply",
            "nextStatus": "applied",
            "occurredAt": "2026-07-15T00:03:00.000Z"
        });
        let response = host.append_review_event_json(&apply.to_string()).unwrap();
        assert_eq!(response.revision, 2);
        assert_eq!(response.operation_state, "accepted");

        let audit = host.get_operation_audit("run-fixture-1").unwrap();
        assert_eq!(audit.run.state, "accepted");
        assert_eq!(audit.lifecycle_events.len(), 7);
        let review = audit.review.unwrap();
        assert_eq!(review.status, "applied");
        assert_eq!(review.events.len(), 2);
        assert_eq!(
            review.events[0].payload.as_ref().unwrap()["source"],
            "inline-review"
        );
    }
}
