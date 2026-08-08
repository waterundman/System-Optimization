use std::collections::{BTreeMap, BTreeSet};
use std::fmt;
use std::fmt::Write as _;
use std::io::{Read, Seek, Write};
use std::path::{Path, PathBuf};

use optimizer_store::{
    AppendReviewEvent, ApplyBlockEdit, ApplyDocumentBatch, BackupManifest, BlockRecord,
    CommitRecord, CreateDocumentWithBlock, CreateKnowledgeItem, CreateReviewCandidateBranch,
    CreateStyleSample, DocumentMutation, DocumentRecord, EditReceipt, KnowledgeItemRecord,
    OperationArtifactKind, OperationRunRecord, OptimizerStore, RestoreSnapshot,
    ReviewCandidateBranchRecord, ReviewCandidateSummaryRecord, ReviewDecision, ReviewEventKind,
    ReviewEventRecord, ReviewSessionRecord, ReviewSessionStatus, SetKnowledgeItemStatus,
    SetStyleSampleStatus, SnapshotRecord, StoreError, StyleSampleRecord,
};
use serde::{Deserialize, Serialize};
use serde_json::{Value, json};
use sha2::{Digest, Sha256};
use time::OffsetDateTime;
use time::format_description::well_known::Rfc3339;
use uuid::Uuid;

const WORKSPACE_SCHEMA_VERSION: u32 = 1;
const MAX_CONTENT_JSON_BYTES: usize = 4 * 1024 * 1024;
const MAX_PLAIN_TEXT_BYTES: usize = 2 * 1024 * 1024;
const MAX_STYLE_SAMPLE_BYTES: usize = 64 * 1024;
const MAX_STYLE_TITLE_CHARS: usize = 120;
const MAX_KNOWLEDGE_CONTENT_BYTES: usize = 64 * 1024;
const MAX_KNOWLEDGE_TITLE_CHARS: usize = 120;

#[derive(Debug, Clone, PartialEq, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct ProjectWorkspace {
    pub schema_version: u32,
    pub project_id: String,
    pub main_branch_id: String,
    pub head_commit_id: String,
    pub revision: i64,
    pub documents: Vec<WorkspaceDocument>,
    pub blocks: Vec<WorkspaceBlock>,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct WorkspaceDocument {
    pub id: String,
    pub parent_id: Option<String>,
    pub kind: String,
    pub title: String,
    pub order_key: String,
    pub revision: i64,
}

#[derive(Debug, Clone, PartialEq, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct WorkspaceBlock {
    pub id: String,
    pub document_id: String,
    pub kind: String,
    pub order_key: String,
    pub content: Value,
    pub plain_text: String,
    pub content_hash: String,
    pub revision: i64,
    pub locked: bool,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct StyleSample {
    pub schema_version: u32,
    pub id: String,
    pub title: String,
    pub content: String,
    pub content_hash: String,
    pub status: String,
    pub sensitivity: String,
    pub revision: i64,
    pub created_at: String,
    pub updated_at: String,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct KnowledgeItem {
    pub schema_version: u32,
    pub id: String,
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

#[derive(Debug, Clone, PartialEq, Eq, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct KnowledgeContextCandidate {
    pub id: String,
    pub source_ref: String,
    pub source_hash: String,
    pub source_commit_id: String,
    pub tier: String,
    pub status: String,
    pub authority: String,
    pub sensitivity: String,
    pub render_mode: String,
    pub reason_codes: Vec<String>,
    pub content: String,
    pub revision: i64,
    pub generated_at: String,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct SummaryInvalidation {
    pub schema_version: u32,
    pub scope_type: String,
    pub scope_id: String,
    pub source_commit_id: String,
    pub reason: String,
    pub invalidation_count: i64,
    pub created_at: String,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct CreateStyleSampleSpec {
    pub title: String,
    pub content: String,
    pub sensitivity: String,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct SetStyleSampleStatusSpec {
    pub id: String,
    pub expected_revision: i64,
    pub status: String,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct CreateKnowledgeItemSpec {
    pub kind: String,
    pub title: String,
    pub content: String,
    pub sensitivity: String,
    pub severity: Option<String>,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct SetKnowledgeItemStatusSpec {
    pub id: String,
    pub expected_revision: i64,
    pub status: String,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct KnowledgeContextSpec {
    pub base_commit_id: String,
    pub target_block_id: String,
    pub target_block_revision: i64,
    pub target_block_hash: String,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct CreateDocumentSpec {
    pub title: String,
    pub initial_text: String,
    pub parent_id: Option<String>,
}

#[derive(Debug, Clone, PartialEq, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct CreateDocumentResponse {
    pub schema_version: u32,
    pub commit_id: String,
    pub previous_head_commit_id: String,
    pub project_revision: i64,
    pub document: WorkspaceDocument,
    pub block: WorkspaceBlock,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct ArchivedDocument {
    pub schema_version: u32,
    pub id: String,
    pub parent_id: Option<String>,
    pub kind: String,
    pub title: String,
    pub order_key: String,
    pub revision: i64,
    pub archived_at: String,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct RenameDocumentSpec {
    pub document_id: String,
    pub expected_revision: i64,
    pub title: String,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum DocumentMoveDirection {
    Up,
    Down,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct ReorderDocumentSpec {
    pub document_id: String,
    pub expected_revision: i64,
    pub direction: DocumentMoveDirection,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum DocumentDepthDirection {
    Indent,
    Outdent,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct ChangeDocumentDepthSpec {
    pub document_id: String,
    pub expected_revision: i64,
    pub direction: DocumentDepthDirection,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct SetDocumentArchivedSpec {
    pub document_id: String,
    pub expected_revision: i64,
    pub archived: bool,
}

#[derive(Debug, Clone, PartialEq, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct DocumentMutationResponse {
    pub schema_version: u32,
    pub commit_id: String,
    pub previous_head_commit_id: String,
    pub head_commit_id: String,
    pub project_revision: i64,
    pub workspace: ProjectWorkspace,
}

#[derive(Debug, Clone, PartialEq)]
pub struct SaveBlockSpec {
    pub block_id: String,
    pub expected_revision: i64,
    pub expected_hash: String,
    pub content: Value,
    pub plain_text: String,
}

#[derive(Debug, Clone, PartialEq, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct SaveBlockResponse {
    pub schema_version: u32,
    pub edit_id: String,
    pub commit_id: String,
    pub previous_head_commit_id: String,
    pub head_commit_id: String,
    pub project_revision: i64,
    pub block: WorkspaceBlock,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct ApplyReviewedProposalSpec {
    pub proposal_id: String,
    pub expected_review_revision: i64,
}

#[derive(Debug, Clone, PartialEq, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct ApplyReviewedProposalResponse {
    pub schema_version: u32,
    pub proposal_id: String,
    pub run_id: String,
    pub review_revision: i64,
    pub accepted_hunks: usize,
    pub rejected_hunks: usize,
    pub save: SaveBlockResponse,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct ReviewCandidateBranch {
    pub proposal_id: String,
    pub branch_id: String,
    pub branch_name: String,
    pub commit_id: String,
    pub snapshot_id: String,
    pub created_at: String,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct ReviewCandidateSummary {
    pub schema_version: u32,
    pub proposal_id: String,
    pub run_id: String,
    pub operation_intent_id: String,
    pub provider_id: String,
    pub model: String,
    pub base_commit_id: String,
    pub target_document_id: String,
    pub target_block_id: String,
    pub hunk_count: usize,
    pub summary: Option<String>,
    pub revision: i64,
    pub status: String,
    pub created_at: String,
    pub updated_at: String,
    pub candidate_branch: Option<ReviewCandidateBranch>,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct ReviewCandidateSession {
    pub proposal_id: String,
    pub proposal_hash: String,
    pub revision: i64,
    pub status: String,
    pub decisions: BTreeMap<String, String>,
}

#[derive(Debug, Clone, PartialEq, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct ReviewCandidateDetail {
    pub schema_version: u32,
    pub summary: ReviewCandidateSummary,
    pub proposal: Value,
    pub session: ReviewCandidateSession,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct CreateReviewCandidateBranchSpec {
    pub proposal_id: String,
    pub expected_review_revision: i64,
    pub branch_name: String,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct CreateReviewCandidateBranchResponse {
    pub schema_version: u32,
    pub branch: ReviewCandidateBranch,
    pub main_head_commit_id: String,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct CheckpointSummary {
    pub id: String,
    pub commit_id: String,
    pub root_hash: String,
    pub codec: String,
    pub codec_version: i64,
    pub checksum: String,
    pub created_at: String,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct VersionCommit {
    pub id: String,
    pub root_hash: String,
    pub reason: String,
    pub actor_type: String,
    pub actor_id: Option<String>,
    pub created_at: String,
    pub parents: Vec<String>,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct VersionHistory {
    pub schema_version: u32,
    pub project_id: String,
    pub head_commit_id: String,
    pub commits: Vec<VersionCommit>,
    pub checkpoints: Vec<CheckpointSummary>,
}

/// v0.6.0 timeline response: a linear, `created_at`-descending view of
/// checkpoints with an optional reference to the operation run that was
/// initiated from each checkpoint's commit.
#[derive(Debug, Clone, PartialEq, Eq, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct TimelineEventsResponse {
    pub schema_version: u32,
    pub events: Vec<TimelineEvent>,
    pub total_count: usize,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct TimelineEvent {
    pub checkpoint_id: String,
    pub commit_id: String,
    pub created_at: String,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub operation_summary: Option<OperationSummary>,
    /// v0.7.0 Stage 1 (D1): the `commit_parent` rows for the event's
    /// commit, ordered by `position` ascending. Exposes branch merge
    /// topology so the frontend SVG overlay can render multi-parent
    /// connection lines. Empty for the seed commit (zero parents) and
    /// single-element for linear commits — both cases stay backward
    /// compatible with the v0.6.0 linear timeline renderer.
    #[serde(default)]
    pub parent_commit_ids: Vec<String>,
}

impl TimelineEvent {
    pub fn checkpoint_id(&self) -> &str {
        &self.checkpoint_id
    }

    pub fn created_at(&self) -> &str {
        &self.created_at
    }
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct OperationSummary {
    pub operation_id: String,
    pub state: String,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub operation_type: Option<String>,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct RestoreCheckpointSpec {
    pub checkpoint_id: String,
}

#[derive(Debug, Clone, PartialEq, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct RestoreCheckpointResponse {
    pub schema_version: u32,
    pub checkpoint_id: String,
    pub commit_id: String,
    pub previous_head_commit_id: String,
    pub restored_root_hash: String,
    pub changed_blocks: usize,
    pub workspace: ProjectWorkspace,
}

#[derive(Debug)]
pub enum WorkspaceCommandError {
    Validation(String),
    DocumentValidation(String),
    StyleValidation(String),
    KnowledgeValidation(String),
    ContextValidation(String),
    InvariantViolation(String),
    NoChanges,
    StoredContent {
        block_id: String,
        source: serde_json::Error,
    },
    Json(serde_json::Error),
    Store(StoreError),
    Clock,
}

impl WorkspaceCommandError {
    pub fn code(&self) -> &'static str {
        match self {
            Self::Validation(_) => "INVALID_BLOCK_EDIT",
            Self::DocumentValidation(_) => "INVALID_DOCUMENT",
            Self::StyleValidation(_) => "INVALID_STYLE_SAMPLE",
            Self::KnowledgeValidation(_) => "INVALID_KNOWLEDGE_ITEM",
            Self::ContextValidation(_) => "INVALID_OPERATION_CONTEXT",
            Self::InvariantViolation(_) => "INVARIANT_VIOLATION",
            Self::NoChanges => "NO_CHANGES",
            Self::StoredContent { .. } => "PROJECT_CONTENT_INVALID",
            Self::Json(_) => "BLOCK_CONTENT_INVALID",
            Self::Store(error) => match error {
                StoreError::Validation(_) => "VALIDATION_FAILED",
                StoreError::NotFound { .. } => "NOT_FOUND",
                StoreError::Conflict { .. } | StoreError::StateConflict { .. } => "CONFLICT",
                StoreError::UnsafeSqliteVersion { .. } => "UNSAFE_STORAGE_VERSION",
                StoreError::Snapshot(_) => "SNAPSHOT_FAILED",
                StoreError::Sqlite(_) | StoreError::InvariantViolation(_) => "STORAGE_FAILED",
            },
            Self::Clock => "HOST_CLOCK_FAILED",
        }
    }

    pub fn public_message(&self) -> String {
        match self {
            Self::Validation(message) => message.clone(),
            Self::DocumentValidation(message) => message.clone(),
            Self::StyleValidation(message) => message.clone(),
            Self::KnowledgeValidation(message) => message.clone(),
            Self::ContextValidation(message) => message.clone(),
            Self::InvariantViolation(message) => message.clone(),
            Self::NoChanges => "Block content has not changed".into(),
            Self::StoredContent { block_id, .. } => {
                format!("Stored content is invalid for block {block_id}")
            }
            Self::Json(_) => "Block content could not be serialized".into(),
            Self::Store(StoreError::Sqlite(_)) => "Storage operation failed".into(),
            Self::Store(StoreError::InvariantViolation(_)) => {
                "Stored project data failed an integrity check".into()
            }
            Self::Store(error) => error.to_string(),
            Self::Clock => "System clock could not create an edit timestamp".into(),
        }
    }
}

impl fmt::Display for WorkspaceCommandError {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        formatter.write_str(&self.public_message())
    }
}

impl std::error::Error for WorkspaceCommandError {
    fn source(&self) -> Option<&(dyn std::error::Error + 'static)> {
        match self {
            Self::StoredContent { source, .. } | Self::Json(source) => Some(source),
            Self::Store(error) => Some(error),
            Self::Validation(_)
            | Self::DocumentValidation(_)
            | Self::StyleValidation(_)
            | Self::KnowledgeValidation(_)
            | Self::ContextValidation(_)
            | Self::InvariantViolation(_)
            | Self::NoChanges
            | Self::Clock => None,
        }
    }
}

impl From<StoreError> for WorkspaceCommandError {
    fn from(value: StoreError) -> Self {
        Self::Store(value)
    }
}

pub(crate) fn load_project_workspace(
    store: &OptimizerStore,
    project_id: &str,
    main_branch_id: &str,
) -> Result<ProjectWorkspace, WorkspaceCommandError> {
    let project = store.get_project(project_id)?;
    let branch = store.get_branch(main_branch_id)?;
    if branch.project_id != project.id || branch.head_commit_id != project.head_commit_id {
        return Err(WorkspaceCommandError::Store(
            StoreError::InvariantViolation("main branch and project head diverged".into()),
        ));
    }
    let documents = store
        .list_documents(project_id)?
        .into_iter()
        .map(workspace_document)
        .collect();
    let blocks = store
        .list_blocks(project_id)?
        .into_iter()
        .map(workspace_block)
        .collect::<Result<_, _>>()?;
    Ok(ProjectWorkspace {
        schema_version: WORKSPACE_SCHEMA_VERSION,
        project_id: project.id,
        main_branch_id: branch.id,
        head_commit_id: project.head_commit_id,
        revision: project.revision,
        documents,
        blocks,
    })
}

pub(crate) fn create_document(
    store: &mut OptimizerStore,
    project_id: &str,
    main_branch_id: &str,
    spec: &CreateDocumentSpec,
) -> Result<CreateDocumentResponse, WorkspaceCommandError> {
    let title = spec.title.trim();
    if title.is_empty() || title.chars().count() > 200 {
        return Err(WorkspaceCommandError::DocumentValidation(
            "Document title must contain 1 to 200 characters".into(),
        ));
    }
    if spec.initial_text.len() > MAX_PLAIN_TEXT_BYTES {
        return Err(WorkspaceCommandError::DocumentValidation(format!(
            "Initial document text exceeds {MAX_PLAIN_TEXT_BYTES} bytes"
        )));
    }
    let project = store.get_project(project_id)?;
    let document_id = generated_id("document");
    let block_id = generated_id("block");
    let commit_id = generated_id("commit");
    let content = json!({
        "type": "paragraph",
        "content": if spec.initial_text.is_empty() {
            Vec::<Value>::new()
        } else {
            vec![json!({ "type": "text", "text": spec.initial_text })]
        },
    });
    let content_json = serde_json::to_string(&content).map_err(WorkspaceCommandError::Json)?;
    let content_hash = block_content_hash("paragraph", &content, &spec.initial_text, false)?;
    let mut documents = store.list_documents(project_id)?;
    if let Some(parent_id) = spec.parent_id.as_deref()
        && !documents.iter().any(|document| document.id == parent_id)
    {
        return Err(WorkspaceCommandError::DocumentValidation(
            "Parent document must be active in the current project".into(),
        ));
    }
    let document_order_key = append_sibling_order_key(&documents, spec.parent_id.as_deref());
    documents.push(DocumentRecord {
        id: document_id.clone(),
        project_id: project_id.to_owned(),
        parent_id: spec.parent_id.clone(),
        kind: "chapter".into(),
        title: title.to_owned(),
        order_key: document_order_key.clone(),
        revision: 0,
        deleted_at: None,
    });
    let mut blocks = store.list_blocks(project_id)?;
    blocks.push(BlockRecord {
        id: block_id.clone(),
        document_id: document_id.clone(),
        kind: "paragraph".into(),
        order_key: "a0".into(),
        content_json: content_json.clone(),
        plain_text: spec.initial_text.clone(),
        content_hash: content_hash.clone(),
        revision: 0,
        locked: false,
    });
    let root_hash = project_root_hash(
        documents.iter(),
        blocks
            .iter()
            .map(|block| (block.id.as_str(), block.content_hash.as_str())),
    );
    let receipt = store.create_document_with_block(&CreateDocumentWithBlock {
        document_id: document_id.clone(),
        document_parent_id: spec.parent_id.clone(),
        document_kind: "chapter".into(),
        document_title: title.to_owned(),
        document_order_key,
        block_id: block_id.clone(),
        block_kind: "paragraph".into(),
        block_order_key: "a0".into(),
        block_content_json: content_json,
        block_plain_text: spec.initial_text.clone(),
        block_content_hash: content_hash,
        block_locked: false,
        commit_id,
        branch_id: main_branch_id.to_owned(),
        expected_head_commit_id: project.head_commit_id,
        expected_project_revision: project.revision,
        new_root_hash: root_hash,
        actor_type: "user".into(),
        actor_id: None,
        occurred_at: now()?,
    })?;
    let document = store
        .list_documents(project_id)?
        .into_iter()
        .find(|item| item.id == receipt.document_id)
        .ok_or_else(|| {
            WorkspaceCommandError::Store(StoreError::InvariantViolation(
                "created document is missing".into(),
            ))
        })?;
    let block = store.get_project_block(project_id, &receipt.block_id)?;
    Ok(CreateDocumentResponse {
        schema_version: WORKSPACE_SCHEMA_VERSION,
        commit_id: receipt.commit_id,
        previous_head_commit_id: receipt.previous_head_commit_id,
        project_revision: receipt.project_revision,
        document: workspace_document(document),
        block: workspace_block(block)?,
    })
}

pub(crate) fn list_archived_documents(
    store: &OptimizerStore,
    project_id: &str,
) -> Result<Vec<ArchivedDocument>, WorkspaceCommandError> {
    store
        .list_archived_documents(project_id)?
        .into_iter()
        .map(|document| {
            let archived_at = document.deleted_at.clone().ok_or_else(|| {
                WorkspaceCommandError::Store(StoreError::InvariantViolation(
                    "archived document has no deleted_at timestamp".into(),
                ))
            })?;
            Ok(ArchivedDocument {
                schema_version: WORKSPACE_SCHEMA_VERSION,
                id: document.id,
                parent_id: document.parent_id,
                kind: document.kind,
                title: document.title,
                order_key: document.order_key,
                revision: document.revision,
                archived_at,
            })
        })
        .collect()
}

pub(crate) fn list_summary_invalidations(
    store: &OptimizerStore,
    project_id: &str,
) -> Result<Vec<SummaryInvalidation>, WorkspaceCommandError> {
    Ok(store
        .list_summary_invalidations(project_id, 1_000)?
        .into_iter()
        .map(|record| SummaryInvalidation {
            schema_version: WORKSPACE_SCHEMA_VERSION,
            scope_type: record.scope_type,
            scope_id: record.scope_id,
            source_commit_id: record.source_commit_id,
            reason: record.reason,
            invalidation_count: record.invalidation_count,
            created_at: record.created_at,
        })
        .collect())
}

pub(crate) fn rename_document(
    store: &mut OptimizerStore,
    project_id: &str,
    main_branch_id: &str,
    spec: &RenameDocumentSpec,
) -> Result<DocumentMutationResponse, WorkspaceCommandError> {
    validate_document_reference(&spec.document_id, spec.expected_revision)?;
    let title = spec.title.trim();
    if title.is_empty() || title.chars().count() > 200 {
        return Err(WorkspaceCommandError::DocumentValidation(
            "Document title must contain 1 to 200 characters".into(),
        ));
    }
    let project = store.get_project(project_id)?;
    let current = store.get_document(project_id, &spec.document_id)?;
    validate_document_revision(&current, spec.expected_revision, &project.head_commit_id)?;
    if current.deleted_at.is_some() {
        return Err(WorkspaceCommandError::DocumentValidation(
            "Archived documents must be restored before renaming".into(),
        ));
    }
    if current.title == title {
        return Err(WorkspaceCommandError::NoChanges);
    }
    let mut next = current.clone();
    next.title = title.to_owned();
    let mut active_documents = store.list_documents(project_id)?;
    let position = active_documents
        .iter()
        .position(|document| document.id == current.id)
        .ok_or_else(|| missing_active_document(&current.id))?;
    active_documents[position] = next.clone();
    commit_document_mutations(
        store,
        project_id,
        main_branch_id,
        &project.head_commit_id,
        project.revision,
        active_documents,
        vec![document_mutation(&current, &next, true, true, "rename")],
        "document_rename",
    )
}

pub(crate) fn reorder_document(
    store: &mut OptimizerStore,
    project_id: &str,
    main_branch_id: &str,
    spec: &ReorderDocumentSpec,
) -> Result<DocumentMutationResponse, WorkspaceCommandError> {
    validate_document_reference(&spec.document_id, spec.expected_revision)?;
    let project = store.get_project(project_id)?;
    let mut current_documents = store.list_documents(project_id)?;
    let current_index = current_documents
        .iter()
        .position(|document| document.id == spec.document_id)
        .ok_or_else(|| missing_active_document(&spec.document_id))?;
    validate_document_revision(
        &current_documents[current_index],
        spec.expected_revision,
        &project.head_commit_id,
    )?;
    let before = current_documents.clone();
    let parent_id = current_documents[current_index].parent_id.clone();
    let mut sibling_ids = ordered_sibling_ids(&current_documents, parent_id.as_deref());
    let sibling_index = sibling_ids
        .iter()
        .position(|id| id == &spec.document_id)
        .ok_or_else(|| invariant_violation("active document is absent from its sibling group"))?;
    let target_index = match spec.direction {
        DocumentMoveDirection::Up => sibling_index.checked_sub(1),
        DocumentMoveDirection::Down => {
            (sibling_index + 1 < sibling_ids.len()).then_some(sibling_index + 1)
        }
    }
    .ok_or(WorkspaceCommandError::NoChanges)?;
    sibling_ids.swap(sibling_index, target_index);
    rekey_documents(&mut current_documents, &sibling_ids);
    let mutations = changed_document_mutations(&before, &current_documents, "reorder");
    if mutations.is_empty() {
        return Err(WorkspaceCommandError::NoChanges);
    }
    commit_document_mutations(
        store,
        project_id,
        main_branch_id,
        &project.head_commit_id,
        project.revision,
        current_documents,
        mutations,
        "document_reorder",
    )
}

pub(crate) fn change_document_depth(
    store: &mut OptimizerStore,
    project_id: &str,
    main_branch_id: &str,
    spec: &ChangeDocumentDepthSpec,
) -> Result<DocumentMutationResponse, WorkspaceCommandError> {
    validate_document_reference(&spec.document_id, spec.expected_revision)?;
    let project = store.get_project(project_id)?;
    let mut documents = store.list_documents(project_id)?;
    let current_index = documents
        .iter()
        .position(|document| document.id == spec.document_id)
        .ok_or_else(|| missing_active_document(&spec.document_id))?;
    validate_document_revision(
        &documents[current_index],
        spec.expected_revision,
        &project.head_commit_id,
    )?;
    let before = documents.clone();
    let current_parent_id = documents[current_index].parent_id.clone();
    let sibling_ids = ordered_sibling_ids(&documents, current_parent_id.as_deref());
    let sibling_index = sibling_ids
        .iter()
        .position(|id| id == &spec.document_id)
        .ok_or_else(|| invariant_violation("active document is absent from its sibling group"))?;

    let destination_ids = match spec.direction {
        DocumentDepthDirection::Indent => {
            let new_parent_id = sibling_index
                .checked_sub(1)
                .and_then(|index| sibling_ids.get(index))
                .cloned()
                .ok_or(WorkspaceCommandError::NoChanges)?;
            documents[current_index].parent_id = Some(new_parent_id.clone());
            let mut children = ordered_sibling_ids(&documents, Some(&new_parent_id));
            children.retain(|id| id != &spec.document_id);
            children.push(spec.document_id.clone());
            children
        }
        DocumentDepthDirection::Outdent => {
            let parent_id = current_parent_id.ok_or(WorkspaceCommandError::NoChanges)?;
            let parent = documents
                .iter()
                .find(|document| document.id == parent_id)
                .cloned()
                .ok_or_else(|| missing_active_document(&parent_id))?;
            documents[current_index].parent_id = parent.parent_id.clone();
            let mut destination = ordered_sibling_ids(&documents, parent.parent_id.as_deref());
            destination.retain(|id| id != &spec.document_id);
            let parent_index = destination
                .iter()
                .position(|id| id == &parent.id)
                .ok_or_else(|| {
                    invariant_violation("active parent is absent from its sibling group")
                })?;
            destination.insert(parent_index + 1, spec.document_id.clone());
            destination
        }
    };
    rekey_documents(&mut documents, &destination_ids);
    let mutations = changed_document_mutations(&before, &documents, "reparent");
    if mutations.is_empty() {
        return Err(WorkspaceCommandError::NoChanges);
    }
    commit_document_mutations(
        store,
        project_id,
        main_branch_id,
        &project.head_commit_id,
        project.revision,
        documents,
        mutations,
        "document_reparent",
    )
}

pub(crate) fn set_document_archived(
    store: &mut OptimizerStore,
    project_id: &str,
    main_branch_id: &str,
    spec: &SetDocumentArchivedSpec,
) -> Result<DocumentMutationResponse, WorkspaceCommandError> {
    validate_document_reference(&spec.document_id, spec.expected_revision)?;
    let project = store.get_project(project_id)?;
    let current = store.get_document(project_id, &spec.document_id)?;
    validate_document_revision(&current, spec.expected_revision, &project.head_commit_id)?;
    let currently_archived = current.deleted_at.is_some();
    if currently_archived == spec.archived {
        return Err(WorkspaceCommandError::NoChanges);
    }
    let mut active_documents = store.list_documents(project_id)?;
    let (mutations, reason) = if spec.archived {
        let subtree_ids = active_subtree_ids(&active_documents, &current.id);
        if active_documents.len() <= subtree_ids.len() {
            return Err(WorkspaceCommandError::DocumentValidation(
                "A project must retain at least one active document".into(),
            ));
        }
        let mutations = active_documents
            .iter()
            .filter(|document| subtree_ids.contains(document.id.as_str()))
            .map(|document| {
                let mut next = document.clone();
                next.deleted_at = Some("archived".into());
                document_mutation(document, &next, true, false, "archive")
            })
            .collect::<Vec<_>>();
        active_documents.retain(|document| !subtree_ids.contains(document.id.as_str()));
        (mutations, "document_archive")
    } else {
        if let Some(parent_id) = current.parent_id.as_deref()
            && !active_documents
                .iter()
                .any(|document| document.id == parent_id)
        {
            return Err(WorkspaceCommandError::DocumentValidation(
                "Restore the parent document before restoring this child".into(),
            ));
        }
        let mut next = current.clone();
        next.deleted_at = None;
        active_documents.push(next.clone());
        active_documents.sort_by(|left, right| {
            left.order_key
                .cmp(&right.order_key)
                .then_with(|| left.id.cmp(&right.id))
        });
        (
            vec![document_mutation(&current, &next, false, true, "restore")],
            "document_restore",
        )
    };
    commit_document_mutations(
        store,
        project_id,
        main_branch_id,
        &project.head_commit_id,
        project.revision,
        active_documents,
        mutations,
        reason,
    )
}

fn append_sibling_order_key(documents: &[DocumentRecord], parent_id: Option<&str>) -> String {
    let suffix = Uuid::new_v4().simple();
    documents
        .iter()
        .filter(|document| document.parent_id.as_deref() == parent_id)
        .map(|document| document.order_key.as_str())
        .max()
        .map_or_else(
            || format!("d-00000000-{suffix}"),
            |last| format!("{last}~{suffix}"),
        )
}

fn ordered_sibling_ids(documents: &[DocumentRecord], parent_id: Option<&str>) -> Vec<String> {
    let mut siblings = documents
        .iter()
        .filter(|document| document.parent_id.as_deref() == parent_id)
        .collect::<Vec<_>>();
    siblings.sort_by(|left, right| {
        left.order_key
            .cmp(&right.order_key)
            .then_with(|| left.id.cmp(&right.id))
    });
    siblings
        .into_iter()
        .map(|document| document.id.clone())
        .collect()
}

fn rekey_documents(documents: &mut [DocumentRecord], ordered_ids: &[String]) {
    // `ordered_ids` is always derived from `documents` itself (via
    // `ordered_sibling_ids`), so every target is provably present; there is
    // no user-controlled input on this path.
    let namespace = Uuid::new_v4().simple();
    for (index, document_id) in ordered_ids.iter().enumerate() {
        let document = documents
            .iter_mut()
            .find(|document| document.id == *document_id)
            .expect("rekey target originates from the current workspace");
        document.order_key = format!("d-{index:08}-{namespace}");
    }
}

fn changed_document_mutations(
    before: &[DocumentRecord],
    after: &[DocumentRecord],
    operation: &str,
) -> Vec<DocumentMutation> {
    // `after` is produced by reordering/reparenting entries copied from
    // `before`, so every id is provably present; no user input reaches here.
    after
        .iter()
        .filter_map(|next| {
            let current = before
                .iter()
                .find(|document| document.id == next.id)
                .expect("changed document originates from the current workspace");
            (current.parent_id != next.parent_id || current.order_key != next.order_key)
                .then(|| document_mutation(current, next, true, true, operation))
        })
        .collect()
}

fn active_subtree_ids(documents: &[DocumentRecord], root_id: &str) -> BTreeSet<String> {
    let mut subtree = BTreeSet::from([root_id.to_owned()]);
    loop {
        let before = subtree.len();
        for document in documents {
            if document
                .parent_id
                .as_deref()
                .is_some_and(|parent_id| subtree.contains(parent_id))
            {
                subtree.insert(document.id.clone());
            }
        }
        if subtree.len() == before {
            return subtree;
        }
    }
}

#[allow(clippy::too_many_arguments)]
fn commit_document_mutations(
    store: &mut OptimizerStore,
    project_id: &str,
    main_branch_id: &str,
    expected_head_commit_id: &str,
    expected_project_revision: i64,
    active_documents: Vec<DocumentRecord>,
    mutations: Vec<DocumentMutation>,
    reason: &str,
) -> Result<DocumentMutationResponse, WorkspaceCommandError> {
    let active_ids = active_documents
        .iter()
        .map(|document| document.id.as_str())
        .collect::<BTreeSet<_>>();
    let blocks = store.list_all_blocks(project_id)?;
    let root_hash = project_root_hash(
        active_documents.iter(),
        blocks
            .iter()
            .filter(|block| active_ids.contains(block.document_id.as_str()))
            .map(|block| (block.id.as_str(), block.content_hash.as_str())),
    );
    let receipt = store.apply_document_batch(&ApplyDocumentBatch {
        mutations,
        commit_id: generated_id("commit"),
        branch_id: main_branch_id.to_owned(),
        expected_head_commit_id: expected_head_commit_id.to_owned(),
        expected_project_revision,
        new_root_hash: root_hash,
        reason: reason.to_owned(),
        actor_type: "user".into(),
        actor_id: None,
        occurred_at: now()?,
    })?;
    let workspace = load_project_workspace(store, project_id, main_branch_id)?;
    Ok(DocumentMutationResponse {
        schema_version: WORKSPACE_SCHEMA_VERSION,
        commit_id: receipt.commit_id.clone(),
        previous_head_commit_id: receipt.previous_head_commit_id,
        head_commit_id: receipt.commit_id,
        project_revision: receipt.project_revision,
        workspace,
    })
}

fn document_mutation(
    current: &DocumentRecord,
    next: &DocumentRecord,
    before_active: bool,
    after_active: bool,
    operation: &str,
) -> DocumentMutation {
    DocumentMutation {
        document_id: current.id.clone(),
        expected_revision: current.revision,
        parent_id: next.parent_id.clone(),
        kind: next.kind.clone(),
        title: next.title.clone(),
        order_key: next.order_key.clone(),
        active: after_active,
        before_hash: document_state_hash(current, before_active),
        after_hash: document_state_hash(next, after_active),
        operation: operation.to_owned(),
    }
}

fn validate_document_reference(
    document_id: &str,
    expected_revision: i64,
) -> Result<(), WorkspaceCommandError> {
    if document_id.trim().is_empty() || expected_revision < 0 {
        return Err(WorkspaceCommandError::DocumentValidation(
            "Document id and non-negative revision are required".into(),
        ));
    }
    Ok(())
}

fn validate_document_revision(
    document: &DocumentRecord,
    expected_revision: i64,
    head_commit_id: &str,
) -> Result<(), WorkspaceCommandError> {
    if document.revision != expected_revision {
        return Err(WorkspaceCommandError::Store(StoreError::StateConflict {
            entity: "document",
            id: document.id.clone(),
            expected_revision,
            actual_revision: document.revision,
            expected_state: head_commit_id.to_owned(),
            actual_state: head_commit_id.to_owned(),
        }));
    }
    Ok(())
}

fn missing_active_document(id: &str) -> WorkspaceCommandError {
    WorkspaceCommandError::Store(StoreError::NotFound {
        entity: "document",
        id: id.to_owned(),
    })
}

fn invariant_violation(message: impl Into<String>) -> WorkspaceCommandError {
    WorkspaceCommandError::InvariantViolation(message.into())
}

pub(crate) fn list_style_samples(
    store: &OptimizerStore,
    project_id: &str,
) -> Result<Vec<StyleSample>, WorkspaceCommandError> {
    store
        .list_style_samples(project_id)?
        .into_iter()
        .map(style_sample)
        .collect::<Result<Vec<_>, _>>()
}

pub(crate) fn create_style_sample(
    store: &mut OptimizerStore,
    project_id: &str,
    spec: &CreateStyleSampleSpec,
) -> Result<StyleSample, WorkspaceCommandError> {
    validate_style_sample(spec)?;
    let title = spec.title.trim().to_owned();
    let content = spec.content.trim().to_owned();
    let content_hash = sha256(content.as_bytes());
    let record = store.create_style_sample(&CreateStyleSample {
        id: generated_id("style"),
        project_id: project_id.to_owned(),
        title,
        content,
        content_hash,
        sensitivity: spec.sensitivity.clone(),
        created_at: now()?,
    })?;
    style_sample(record)
}

pub(crate) fn set_style_sample_status(
    store: &mut OptimizerStore,
    project_id: &str,
    spec: &SetStyleSampleStatusSpec,
) -> Result<StyleSample, WorkspaceCommandError> {
    if spec.id.trim().is_empty() || spec.expected_revision < 0 {
        return Err(WorkspaceCommandError::StyleValidation(
            "Style sample id and non-negative revision are required".into(),
        ));
    }
    if !matches!(spec.status.as_str(), "canonical" | "archived") {
        return Err(WorkspaceCommandError::StyleValidation(
            "Style sample status must be canonical or archived".into(),
        ));
    }
    let record = store.set_style_sample_status(&SetStyleSampleStatus {
        project_id: project_id.to_owned(),
        id: spec.id.clone(),
        expected_revision: spec.expected_revision,
        status: spec.status.clone(),
        updated_at: now()?,
    })?;
    style_sample(record)
}

pub(crate) fn list_knowledge_items(
    store: &OptimizerStore,
    project_id: &str,
) -> Result<Vec<KnowledgeItem>, WorkspaceCommandError> {
    store
        .list_knowledge_items(project_id)?
        .into_iter()
        .map(knowledge_item)
        .collect::<Result<Vec<_>, _>>()
}

pub(crate) fn create_knowledge_item(
    store: &mut OptimizerStore,
    project_id: &str,
    spec: &CreateKnowledgeItemSpec,
) -> Result<KnowledgeItem, WorkspaceCommandError> {
    validate_knowledge_item(spec)?;
    let title = spec.title.trim().to_owned();
    let content = spec.content.trim().to_owned();
    let record = store.create_knowledge_item(&CreateKnowledgeItem {
        id: generated_id("knowledge"),
        project_id: project_id.to_owned(),
        kind: spec.kind.clone(),
        title,
        content_hash: sha256(content.as_bytes()),
        content,
        authority: "user_confirmed".into(),
        sensitivity: spec.sensitivity.clone(),
        severity: spec.severity.clone(),
        created_at: now()?,
    })?;
    knowledge_item(record)
}

pub(crate) fn set_knowledge_item_status(
    store: &mut OptimizerStore,
    project_id: &str,
    spec: &SetKnowledgeItemStatusSpec,
) -> Result<KnowledgeItem, WorkspaceCommandError> {
    if spec.id.trim().is_empty() || spec.expected_revision < 0 {
        return Err(WorkspaceCommandError::KnowledgeValidation(
            "Knowledge item id and non-negative revision are required".into(),
        ));
    }
    if !matches!(spec.status.as_str(), "canonical" | "archived" | "rejected") {
        return Err(WorkspaceCommandError::KnowledgeValidation(
            "Knowledge item status must be canonical, archived or rejected".into(),
        ));
    }
    let record = store.set_knowledge_item_status(&SetKnowledgeItemStatus {
        project_id: project_id.to_owned(),
        id: spec.id.clone(),
        expected_revision: spec.expected_revision,
        status: spec.status.clone(),
        updated_at: now()?,
    })?;
    knowledge_item(record)
}

pub(crate) fn knowledge_context(
    store: &OptimizerStore,
    project_id: &str,
    spec: &KnowledgeContextSpec,
) -> Result<Vec<KnowledgeContextCandidate>, WorkspaceCommandError> {
    let project = store.get_project(project_id)?;
    if spec.base_commit_id.trim().is_empty() || spec.base_commit_id != project.head_commit_id {
        return Err(WorkspaceCommandError::KnowledgeValidation(
            "Knowledge context must bind to the current project HEAD".into(),
        ));
    }
    let target = store.get_project_block(project_id, &spec.target_block_id)?;
    if target.revision != spec.target_block_revision
        || target.content_hash != spec.target_block_hash
    {
        return Err(WorkspaceCommandError::KnowledgeValidation(
            "Knowledge context target changed before collection".into(),
        ));
    }
    store
        .list_knowledge_items(project_id)?
        .into_iter()
        .filter(|record| record.status == "canonical")
        .map(|record| knowledge_context_candidate(record, &project.head_commit_id))
        .collect()
}

pub(crate) fn save_block(
    store: &mut OptimizerStore,
    project_id: &str,
    main_branch_id: &str,
    spec: &SaveBlockSpec,
) -> Result<SaveBlockResponse, WorkspaceCommandError> {
    validate_save_spec(spec)?;
    let project = store.get_project(project_id)?;
    let current = store.get_project_block(project_id, &spec.block_id)?;
    if current.revision != spec.expected_revision || current.content_hash != spec.expected_hash {
        return Err(WorkspaceCommandError::Store(StoreError::Conflict {
            entity: "block",
            id: current.id,
            expected_revision: spec.expected_revision,
            actual_revision: current.revision,
            expected_hash: spec.expected_hash.clone(),
            actual_hash: current.content_hash,
        }));
    }
    let content_json = serde_json::to_string(&spec.content).map_err(WorkspaceCommandError::Json)?;
    if content_json.len() > MAX_CONTENT_JSON_BYTES {
        return Err(WorkspaceCommandError::Validation(format!(
            "Block content exceeds {MAX_CONTENT_JSON_BYTES} bytes"
        )));
    }
    let content_hash = block_content_hash(
        &current.kind,
        &spec.content,
        &spec.plain_text,
        current.locked,
    )?;
    if content_hash == current.content_hash
        && content_json == current.content_json
        && spec.plain_text == current.plain_text
    {
        return Err(WorkspaceCommandError::NoChanges);
    }
    let documents = store.list_documents(project_id)?;
    let blocks = store.list_blocks(project_id)?;
    let root_hash = project_root_hash(
        documents.iter(),
        blocks.iter().map(|block| {
            (
                block.id.as_str(),
                if block.id == current.id {
                    content_hash.as_str()
                } else {
                    block.content_hash.as_str()
                },
            )
        }),
    );
    let command = ApplyBlockEdit {
        edit_id: generated_id("edit"),
        commit_id: generated_id("commit"),
        branch_id: main_branch_id.into(),
        expected_head_commit_id: project.head_commit_id,
        expected_project_revision: project.revision,
        block_id: spec.block_id.clone(),
        expected_revision: spec.expected_revision,
        expected_hash: spec.expected_hash.clone(),
        new_content_json: content_json,
        new_plain_text: spec.plain_text.clone(),
        new_content_hash: content_hash,
        new_root_hash: root_hash,
        reason: "autosave".into(),
        actor_type: "user".into(),
        actor_id: None,
        occurred_at: now()?,
        review_event: None,
    };
    let receipt = store.apply_block_edit(&command)?;
    save_response(store, project_id, receipt)
}

pub(crate) fn list_review_candidates(
    store: &OptimizerStore,
    project_id: &str,
) -> Result<Vec<ReviewCandidateSummary>, WorkspaceCommandError> {
    store
        .list_review_candidates(project_id, 100)?
        .into_iter()
        .map(review_candidate_summary)
        .collect()
}

pub(crate) fn load_review_candidate(
    store: &OptimizerStore,
    project_id: &str,
    proposal_id: &str,
) -> Result<ReviewCandidateDetail, WorkspaceCommandError> {
    if proposal_id.trim().is_empty() || proposal_id.len() > 200 {
        return Err(WorkspaceCommandError::Validation(
            "Proposal id is invalid".into(),
        ));
    }
    let loaded = load_stored_review_candidate(store, project_id, proposal_id)?;
    let decisions = loaded
        .decisions
        .into_iter()
        .map(|(id, decision)| {
            (
                id,
                decision
                    .map(|value| value.as_str().to_owned())
                    .unwrap_or_else(|| "pending".into()),
            )
        })
        .collect();
    Ok(ReviewCandidateDetail {
        schema_version: WORKSPACE_SCHEMA_VERSION,
        summary: review_candidate_summary(loaded.summary)?,
        proposal: loaded.payload,
        session: ReviewCandidateSession {
            proposal_id: loaded.session.proposal_id,
            proposal_hash: loaded.proposal.proposal_hash,
            revision: loaded.session.revision,
            status: loaded.session.status.as_str().into(),
            decisions,
        },
    })
}

pub(crate) fn apply_reviewed_proposal(
    store: &mut OptimizerStore,
    project_id: &str,
    main_branch_id: &str,
    spec: &ApplyReviewedProposalSpec,
) -> Result<ApplyReviewedProposalResponse, WorkspaceCommandError> {
    if spec.proposal_id.trim().is_empty()
        || spec.proposal_id.len() > 200
        || spec.expected_review_revision < 0
    {
        return Err(WorkspaceCommandError::Validation(
            "Proposal id and expected review revision are invalid".into(),
        ));
    }
    let session = store.get_review_session(&spec.proposal_id)?;
    if session.revision != spec.expected_review_revision
        || session.status != ReviewSessionStatus::Ready
    {
        return Err(WorkspaceCommandError::Store(StoreError::StateConflict {
            entity: "patch_review",
            id: spec.proposal_id.clone(),
            expected_revision: spec.expected_review_revision,
            actual_revision: session.revision,
            expected_state: ReviewSessionStatus::Ready.as_str().into(),
            actual_state: session.status.as_str().into(),
        }));
    }
    let run = store.get_operation_run(&session.run_id)?;
    if run.project_id != project_id {
        return Err(WorkspaceCommandError::Validation(
            "Proposal belongs to another project".into(),
        ));
    }
    let artifact = store.get_operation_artifact(&run.id)?;
    if artifact.id != spec.proposal_id || artifact.kind != OperationArtifactKind::PatchProposal {
        return Err(WorkspaceCommandError::Validation(
            "Proposal artifact binding is invalid".into(),
        ));
    }
    let payload: Value =
        serde_json::from_str(&artifact.payload_json).map_err(WorkspaceCommandError::Json)?;
    verify_stored_proposal_hash(&payload, &artifact.binding_hash)?;
    let proposal: StoredPatchProposal =
        serde_json::from_value(payload).map_err(WorkspaceCommandError::Json)?;
    if proposal.schema_version != 2
        || proposal.id != spec.proposal_id
        || proposal.operation_run_id != run.id
        || proposal.base_commit_id != run.base_commit_id
        || proposal.proposal_hash != artifact.binding_hash
        || proposal.status != "review"
    {
        return Err(WorkspaceCommandError::Validation(
            "Stored proposal metadata is not bound to its operation".into(),
        ));
    }

    let project = store.get_project(project_id)?;
    if project.head_commit_id != run.base_commit_id {
        return Err(WorkspaceCommandError::Store(StoreError::StateConflict {
            entity: "operation_base_commit",
            id: run.id.clone(),
            expected_revision: spec.expected_review_revision,
            actual_revision: session.revision,
            expected_state: run.base_commit_id.clone(),
            actual_state: project.head_commit_id,
        }));
    }
    let current = store.get_project_block(project_id, &proposal.target.block_id)?;
    if current.document_id != proposal.target.document_id
        || current.revision != proposal.target.base_revision
        || current.content_hash != proposal.target.base_hash
        || current.locked
        || proposal.target.from.block_id != current.id
        || proposal.target.to.block_id != current.id
        || proposal.target.from.offset > proposal.target.to.offset
    {
        return Err(WorkspaceCommandError::Store(StoreError::Conflict {
            entity: "proposal_target",
            id: current.id,
            expected_revision: proposal.target.base_revision,
            actual_revision: current.revision,
            expected_hash: proposal.target.base_hash,
            actual_hash: current.content_hash,
        }));
    }

    let events = store.list_review_events(&spec.proposal_id)?;
    validate_proposal_hunks(&current.plain_text, &proposal.target, &proposal.hunks)?;
    let decisions = complete_review_decisions(
        &proposal.hunks,
        replay_review_decisions(&proposal.hunks, &events)?,
    )?;
    validate_atomic_decisions(&proposal.hunks, &decisions)?;

    let mut accepted = Vec::new();
    let mut rejected = Vec::new();
    for hunk in &proposal.hunks {
        let decision = decisions
            .get(&hunk.id)
            .ok_or_else(|| invariant_violation("review decisions must cover every proposal hunk"))?;
        match decision {
            ReviewDecision::Accepted => accepted.push(hunk),
            ReviewDecision::Rejected => rejected.push(hunk.id.clone()),
        }
    }
    if accepted.is_empty() {
        return Err(WorkspaceCommandError::Validation(
            "A proposal with no accepted hunks must be rejected, not applied".into(),
        ));
    }
    let next_plain_text = apply_proposal_hunks(&current.plain_text, &accepted)?;
    if next_plain_text.len() > MAX_PLAIN_TEXT_BYTES {
        return Err(WorkspaceCommandError::Validation(format!(
            "Block plain text exceeds {MAX_PLAIN_TEXT_BYTES} bytes"
        )));
    }
    let mut next_content: Value =
        serde_json::from_str(&current.content_json).map_err(WorkspaceCommandError::Json)?;
    let content = next_content.as_object_mut().ok_or_else(|| {
        WorkspaceCommandError::Validation("Stored Block content must be an object".into())
    })?;
    content.remove("text");
    content.insert(
        "content".into(),
        if next_plain_text.is_empty() {
            Value::Array(Vec::new())
        } else {
            json!([{ "type": "text", "text": next_plain_text }])
        },
    );
    let content_json = serde_json::to_string(&next_content).map_err(WorkspaceCommandError::Json)?;
    let content_hash = block_content_hash(
        &current.kind,
        &next_content,
        &next_plain_text,
        current.locked,
    )?;
    let documents = store.list_documents(project_id)?;
    let blocks = store.list_blocks(project_id)?;
    let root_hash = project_root_hash(
        documents.iter(),
        blocks.iter().map(|block| {
            (
                block.id.as_str(),
                if block.id == current.id {
                    content_hash.as_str()
                } else {
                    block.content_hash.as_str()
                },
            )
        }),
    );
    let occurred_at = now()?;
    let commit_id = generated_id("commit");
    let accepted_ids = accepted
        .iter()
        .map(|hunk| hunk.id.clone())
        .collect::<Vec<_>>();
    let review_event = AppendReviewEvent {
        id: generated_id("review-event"),
        proposal_id: spec.proposal_id.clone(),
        expected_revision: spec.expected_review_revision,
        expected_status: ReviewSessionStatus::Ready,
        kind: ReviewEventKind::Apply,
        next_status: ReviewSessionStatus::Applied,
        hunk_id: None,
        decision: None,
        payload_json: Some(
            serde_json::to_string(&json!({
                "commitId": commit_id,
                "acceptedHunkIds": accepted_ids.clone(),
                "rejectedHunkIds": rejected.clone(),
            }))
            .map_err(WorkspaceCommandError::Json)?,
        ),
        occurred_at: occurred_at.clone(),
    };
    let command = ApplyBlockEdit {
        edit_id: generated_id("edit"),
        commit_id,
        branch_id: main_branch_id.into(),
        expected_head_commit_id: run.base_commit_id,
        expected_project_revision: project.revision,
        block_id: current.id,
        expected_revision: current.revision,
        expected_hash: current.content_hash,
        new_content_json: content_json,
        new_plain_text: next_plain_text,
        new_content_hash: content_hash,
        new_root_hash: root_hash,
        reason: "ai_accept".into(),
        actor_type: "model".into(),
        actor_id: Some(run.id.clone()),
        occurred_at,
        review_event: Some(review_event),
    };
    let receipt = store.apply_block_edit(&command)?;
    Ok(ApplyReviewedProposalResponse {
        schema_version: WORKSPACE_SCHEMA_VERSION,
        proposal_id: spec.proposal_id.clone(),
        run_id: run.id,
        review_revision: spec.expected_review_revision + 1,
        accepted_hunks: accepted_ids.len(),
        rejected_hunks: rejected.len(),
        save: save_response(store, project_id, receipt)?,
    })
}

pub(crate) fn create_review_candidate_branch(
    store: &mut OptimizerStore,
    project_id: &str,
    spec: &CreateReviewCandidateBranchSpec,
) -> Result<CreateReviewCandidateBranchResponse, WorkspaceCommandError> {
    let branch_name = spec.branch_name.trim();
    if spec.proposal_id.trim().is_empty()
        || spec.proposal_id.len() > 200
        || spec.expected_review_revision < 0
        || branch_name.is_empty()
        || branch_name.chars().count() > 120
    {
        return Err(WorkspaceCommandError::Validation(
            "Candidate proposal, review revision and branch name are invalid".into(),
        ));
    }
    let loaded = load_stored_review_candidate(store, project_id, &spec.proposal_id)?;
    if loaded.session.revision != spec.expected_review_revision
        || loaded.session.status != ReviewSessionStatus::Ready
    {
        return Err(WorkspaceCommandError::Store(StoreError::StateConflict {
            entity: "patch_review",
            id: spec.proposal_id.clone(),
            expected_revision: spec.expected_review_revision,
            actual_revision: loaded.session.revision,
            expected_state: ReviewSessionStatus::Ready.as_str().into(),
            actual_state: loaded.session.status.as_str().into(),
        }));
    }
    if loaded.summary.candidate_branch.is_some() {
        return Err(WorkspaceCommandError::Validation(
            "Candidate already has a branch".into(),
        ));
    }
    let project = store.get_project(project_id)?;
    if project.head_commit_id != loaded.run.base_commit_id {
        return Err(WorkspaceCommandError::Store(StoreError::StateConflict {
            entity: "candidate_branch_base",
            id: spec.proposal_id.clone(),
            expected_revision: spec.expected_review_revision,
            actual_revision: loaded.session.revision,
            expected_state: loaded.run.base_commit_id,
            actual_state: project.head_commit_id,
        }));
    }
    let current = store.get_project_block(project_id, &loaded.proposal.target.block_id)?;
    if current.document_id != loaded.proposal.target.document_id
        || current.revision != loaded.proposal.target.base_revision
        || current.content_hash != loaded.proposal.target.base_hash
        || current.locked
    {
        return Err(WorkspaceCommandError::Store(StoreError::Conflict {
            entity: "candidate_branch_target",
            id: current.id,
            expected_revision: loaded.proposal.target.base_revision,
            actual_revision: current.revision,
            expected_hash: loaded.proposal.target.base_hash,
            actual_hash: current.content_hash,
        }));
    }
    validate_proposal_hunks(
        &current.plain_text,
        &loaded.proposal.target,
        &loaded.proposal.hunks,
    )?;
    let decisions = complete_review_decisions(&loaded.proposal.hunks, loaded.decisions)?;
    validate_atomic_decisions(&loaded.proposal.hunks, &decisions)?;
    let accepted = loaded
        .proposal
        .hunks
        .iter()
        .filter(|hunk| decisions.get(&hunk.id) == Some(&ReviewDecision::Accepted))
        .collect::<Vec<_>>();
    if accepted.is_empty() {
        return Err(WorkspaceCommandError::Validation(
            "A candidate branch requires at least one accepted hunk".into(),
        ));
    }
    let next_plain_text = apply_proposal_hunks(&current.plain_text, &accepted)?;
    if next_plain_text.len() > MAX_PLAIN_TEXT_BYTES {
        return Err(WorkspaceCommandError::Validation(format!(
            "Block plain text exceeds {MAX_PLAIN_TEXT_BYTES} bytes"
        )));
    }
    let mut next_content: Value =
        serde_json::from_str(&current.content_json).map_err(WorkspaceCommandError::Json)?;
    let content = next_content.as_object_mut().ok_or_else(|| {
        WorkspaceCommandError::Validation("Stored Block content must be an object".into())
    })?;
    content.remove("text");
    content.insert(
        "content".into(),
        if next_plain_text.is_empty() {
            Value::Array(Vec::new())
        } else {
            json!([{ "type": "text", "text": next_plain_text }])
        },
    );
    let content_json = serde_json::to_string(&next_content).map_err(WorkspaceCommandError::Json)?;
    let content_hash = block_content_hash(
        &current.kind,
        &next_content,
        &next_plain_text,
        current.locked,
    )?;
    let documents = store.list_documents(project_id)?;
    let blocks = store.list_blocks(project_id)?;
    let root_hash = project_root_hash(
        documents.iter(),
        blocks.iter().map(|block| {
            (
                block.id.as_str(),
                if block.id == current.id {
                    content_hash.as_str()
                } else {
                    block.content_hash.as_str()
                },
            )
        }),
    );
    let occurred_at = now()?;
    let branch = store.create_review_candidate_branch(&CreateReviewCandidateBranch {
        proposal_id: spec.proposal_id.clone(),
        expected_review_revision: spec.expected_review_revision,
        project_id: project_id.into(),
        expected_project_head_commit_id: project.head_commit_id.clone(),
        branch_id: generated_id("branch"),
        branch_name: branch_name.into(),
        commit_id: generated_id("commit"),
        snapshot_id: generated_id("snapshot"),
        target_document_id: current.document_id,
        target_block_id: current.id,
        expected_block_revision: current.revision,
        expected_block_hash: current.content_hash,
        new_content_json: content_json,
        new_plain_text: next_plain_text,
        new_content_hash: content_hash,
        new_root_hash: root_hash,
        occurred_at,
    })?;
    Ok(CreateReviewCandidateBranchResponse {
        schema_version: WORKSPACE_SCHEMA_VERSION,
        branch: review_candidate_branch(branch),
        main_head_commit_id: project.head_commit_id,
    })
}

pub(crate) fn create_checkpoint(
    store: &mut OptimizerStore,
    project_id: &str,
) -> Result<CheckpointSummary, WorkspaceCommandError> {
    let created_at = now()?;
    let record = store.create_head_snapshot(&generated_id("snapshot"), project_id, &created_at)?;
    Ok(checkpoint_summary(record))
}

pub(crate) fn load_version_history(
    store: &OptimizerStore,
    project_id: &str,
) -> Result<VersionHistory, WorkspaceCommandError> {
    let project = store.get_project(project_id)?;
    let commits = store
        .list_commits(project_id)?
        .into_iter()
        .map(version_commit)
        .collect();
    let checkpoints = store
        .list_snapshots(project_id)?
        .into_iter()
        .map(checkpoint_summary)
        .collect();
    Ok(VersionHistory {
        schema_version: WORKSPACE_SCHEMA_VERSION,
        project_id: project.id,
        head_commit_id: project.head_commit_id,
        commits,
        checkpoints,
    })
}

/// v0.6.0 timeline: derive a linear, `created_at`-descending event stream
/// from the existing version history checkpoints, optionally associating
/// each checkpoint with the operation run that was initiated from its
/// commit. Reuses `load_version_history` (D4) so the data source stays
/// identical to the version drawer; operation runs are matched via
/// `operation_run.base_commit_id == checkpoint.commit_id`.
///
/// v0.7.0 Stage 1 (D1): each event now also carries its
/// [`parent_commit_ids`](TimelineEvent::parent_commit_ids) — the ordered
/// list of parent commit ids drawn from the existing `commit_parent` table
/// (no schema change). Multi-parent entries surface branch merge points so
/// the frontend SVG overlay can render connection lines without introducing
/// CRDT state. Empty for the seed commit and single-element for linear
/// commits, preserving the v0.6.0 backward-compatible baseline.
pub fn list_timeline_events(
    store: &OptimizerStore,
    project_id: &str,
) -> Result<TimelineEventsResponse, WorkspaceCommandError> {
    let history = load_version_history(store, project_id)?;
    // list_operation_runs orders by started_at DESC, so the first run seen
    // for a given base_commit_id is the latest one. Cap at the store's max
    // page size (200) — timelines cover recent activity, not full archives.
    let runs = store
        .list_operation_runs(project_id, 200, 0)
        .map_err(WorkspaceCommandError::Store)?;
    let mut latest_run_by_base: BTreeMap<&str, &OperationRunRecord> = BTreeMap::new();
    for run in &runs {
        latest_run_by_base
            .entry(run.base_commit_id.as_str())
            .or_insert(run);
    }
    // v0.7.0 Stage 1 (D1): preload each checkpoint commit's parent ids via a
    // single batched `IN (...)` query (chunks of at most 200 ids) instead of
    // one query per commit (N+1). Empty for the seed commit and single-element
    // for linear commits — both cases stay backward compatible with the
    // v0.6.0 linear timeline renderer.
    let mut distinct_commit_ids: Vec<String> = history
        .checkpoints
        .iter()
        .map(|checkpoint| checkpoint.commit_id.clone())
        .collect::<BTreeSet<_>>()
        .into_iter()
        .collect();
    distinct_commit_ids.sort();
    let parents_by_commit = store
        .list_commit_parents_batch(project_id, &distinct_commit_ids)
        .map_err(WorkspaceCommandError::Store)?;
    let mut events: Vec<TimelineEvent> = history
        .checkpoints
        .into_iter()
        .map(|checkpoint| {
            let operation_summary = latest_run_by_base
                .get(checkpoint.commit_id.as_str())
                .map(|run| OperationSummary {
                    operation_id: run.id.clone(),
                    state: run.state.as_str().to_owned(),
                    operation_type: None,
                });
            let parent_commit_ids = parents_by_commit
                .get(&checkpoint.commit_id)
                .cloned()
                .unwrap_or_default();
            TimelineEvent {
                checkpoint_id: checkpoint.id,
                commit_id: checkpoint.commit_id,
                created_at: checkpoint.created_at,
                operation_summary,
                parent_commit_ids,
            }
        })
        .collect();
    events.sort_by(|a, b| b.created_at.cmp(&a.created_at));
    let total_count = events.len();
    Ok(TimelineEventsResponse {
        schema_version: WORKSPACE_SCHEMA_VERSION,
        events,
        total_count,
    })
}

pub(crate) fn restore_checkpoint(
    store: &mut OptimizerStore,
    project_id: &str,
    main_branch_id: &str,
    spec: &RestoreCheckpointSpec,
) -> Result<RestoreCheckpointResponse, WorkspaceCommandError> {
    if spec.checkpoint_id.trim().is_empty() || spec.checkpoint_id.len() > 200 {
        return Err(WorkspaceCommandError::Validation(
            "Checkpoint id is invalid".into(),
        ));
    }
    let receipt = store.restore_snapshot(&RestoreSnapshot {
        snapshot_id: spec.checkpoint_id.clone(),
        branch_id: main_branch_id.into(),
        new_commit_id: generated_id("commit"),
        edit_id_prefix: generated_id("restore-edit"),
        actor_type: "user".into(),
        actor_id: None,
        occurred_at: now()?,
    })?;
    let workspace = load_project_workspace(store, project_id, main_branch_id)?;
    Ok(RestoreCheckpointResponse {
        schema_version: WORKSPACE_SCHEMA_VERSION,
        checkpoint_id: receipt.snapshot_id,
        commit_id: receipt.commit_id,
        previous_head_commit_id: receipt.previous_head_commit_id,
        restored_root_hash: receipt.restored_root_hash,
        changed_blocks: receipt.changed_blocks,
        workspace,
    })
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
pub struct CompareDocumentsSpec {
    pub snapshot_id_a: String,
    pub snapshot_id_b: String,
    pub document_id_a: String,
    pub document_id_b: String,
}

pub(crate) fn compare_documents(
    store: &OptimizerStore,
    project_id: &str,
    spec: &CompareDocumentsSpec,
) -> Result<optimizer_store::DocumentDiffResult, WorkspaceCommandError> {
    for (field, value) in [
        ("snapshotIdA", spec.snapshot_id_a.as_str()),
        ("snapshotIdB", spec.snapshot_id_b.as_str()),
        ("documentIdA", spec.document_id_a.as_str()),
        ("documentIdB", spec.document_id_b.as_str()),
    ] {
        if value.trim().is_empty() || value.len() > 200 {
            return Err(WorkspaceCommandError::Validation(format!(
                "{field} must be a non-empty string of at most 200 characters"
            )));
        }
    }

    let record_a = store.get_snapshot(&spec.snapshot_id_a)?;
    if record_a.project_id != project_id {
        return Err(WorkspaceCommandError::Validation(
            "snapshotIdA does not belong to the current project".into(),
        ));
    }
    let record_b = store.get_snapshot(&spec.snapshot_id_b)?;
    if record_b.project_id != project_id {
        return Err(WorkspaceCommandError::Validation(
            "snapshotIdB does not belong to the current project".into(),
        ));
    }

    let snapshot_a = store.decode_snapshot_record(&record_a)?;
    let snapshot_b = store.decode_snapshot_record(&record_b)?;

    Ok(optimizer_store::compare_documents(
        &snapshot_a,
        &snapshot_b,
        &spec.document_id_a,
        &spec.document_id_b,
    ))
}

#[derive(Debug, Deserialize)]
#[serde(rename_all = "camelCase")]
struct StoredPatchProposal {
    schema_version: u32,
    id: String,
    operation_run_id: String,
    base_commit_id: String,
    target: StoredProposalTarget,
    hunks: Vec<StoredProposalHunk>,
    status: String,
    proposal_hash: String,
}

#[derive(Debug, Deserialize)]
#[serde(rename_all = "camelCase")]
struct StoredProposalTarget {
    document_id: String,
    block_id: String,
    base_revision: i64,
    base_hash: String,
    from: StoredTextAnchor,
    to: StoredTextAnchor,
}

#[derive(Debug, Deserialize)]
#[serde(rename_all = "camelCase")]
struct StoredProposalHunk {
    id: String,
    from: StoredTextAnchor,
    to: StoredTextAnchor,
    original: String,
    replacement: String,
    atomic_group: Option<String>,
}

#[derive(Debug, Deserialize)]
#[serde(rename_all = "camelCase")]
struct StoredTextAnchor {
    block_id: String,
    offset: usize,
}

struct LoadedStoredReviewCandidate {
    summary: ReviewCandidateSummaryRecord,
    session: ReviewSessionRecord,
    run: OperationRunRecord,
    payload: Value,
    proposal: StoredPatchProposal,
    decisions: BTreeMap<String, Option<ReviewDecision>>,
}

fn load_stored_review_candidate(
    store: &OptimizerStore,
    project_id: &str,
    proposal_id: &str,
) -> Result<LoadedStoredReviewCandidate, WorkspaceCommandError> {
    let summary = store.get_review_candidate_summary(proposal_id)?;
    if summary.project_id != project_id {
        return Err(WorkspaceCommandError::Validation(
            "Proposal belongs to another project".into(),
        ));
    }
    let session = store.get_review_session(proposal_id)?;
    let run = store.get_operation_run(&session.run_id)?;
    let artifact = store.get_operation_artifact_by_id(proposal_id)?;
    if session.run_id != summary.run_id
        || run.id != summary.run_id
        || run.project_id != project_id
        || artifact.run_id != run.id
        || artifact.kind != OperationArtifactKind::PatchProposal
    {
        return Err(WorkspaceCommandError::Store(
            StoreError::InvariantViolation(
                "review candidate store bindings are inconsistent".into(),
            ),
        ));
    }
    let payload: Value =
        serde_json::from_str(&artifact.payload_json).map_err(WorkspaceCommandError::Json)?;
    verify_stored_proposal_hash(&payload, &artifact.binding_hash)?;
    let proposal: StoredPatchProposal =
        serde_json::from_value(payload.clone()).map_err(WorkspaceCommandError::Json)?;
    if proposal.schema_version != 2
        || proposal.id != proposal_id
        || proposal.operation_run_id != run.id
        || proposal.base_commit_id != run.base_commit_id
        || proposal.proposal_hash != artifact.binding_hash
        || proposal.status != "review"
        || proposal.target.document_id != summary.target_document_id
        || proposal.target.block_id != summary.target_block_id
        || proposal.hunks.len() != usize::try_from(summary.hunk_count).unwrap_or(usize::MAX)
    {
        return Err(WorkspaceCommandError::Validation(
            "Stored proposal metadata is not bound to its candidate summary".into(),
        ));
    }
    let events = store.list_review_events(proposal_id)?;
    let decisions = replay_review_decisions(&proposal.hunks, &events)?;
    let all_decided = decisions.values().all(Option::is_some);
    if matches!(
        session.status,
        ReviewSessionStatus::Ready | ReviewSessionStatus::Applied
    ) && !all_decided
        || session.status == ReviewSessionStatus::Review && all_decided
    {
        return Err(WorkspaceCommandError::Store(
            StoreError::InvariantViolation(
                "review candidate status disagrees with replayed decisions".into(),
            ),
        ));
    }
    Ok(LoadedStoredReviewCandidate {
        summary,
        session,
        run,
        payload,
        proposal,
        decisions,
    })
}

fn replay_review_decisions(
    hunks: &[StoredProposalHunk],
    events: &[ReviewEventRecord],
) -> Result<BTreeMap<String, Option<ReviewDecision>>, WorkspaceCommandError> {
    let mut decisions = hunks
        .iter()
        .map(|hunk| (hunk.id.clone(), None))
        .collect::<BTreeMap<_, _>>();
    for event in events {
        if event.kind != ReviewEventKind::Decision {
            continue;
        }
        let hunk_id = event.hunk_id.as_deref().ok_or_else(|| {
            WorkspaceCommandError::Validation("Decision event has no hunk id".into())
        })?;
        let decision = event.decision.ok_or_else(|| {
            WorkspaceCommandError::Validation("Decision event has no decision".into())
        })?;
        let selected = hunks
            .iter()
            .find(|hunk| hunk.id == hunk_id)
            .ok_or_else(|| {
                WorkspaceCommandError::Validation(
                    "Decision event references an unknown hunk".into(),
                )
            })?;
        if let Some(group) = selected.atomic_group.as_deref() {
            if group.trim().is_empty() {
                return Err(WorkspaceCommandError::Validation(
                    "Atomic proposal group cannot be empty".into(),
                ));
            }
            for hunk in hunks
                .iter()
                .filter(|hunk| hunk.atomic_group.as_deref() == Some(group))
            {
                decisions.insert(hunk.id.clone(), Some(decision));
            }
        } else {
            decisions.insert(selected.id.clone(), Some(decision));
        }
    }
    Ok(decisions)
}

fn complete_review_decisions(
    hunks: &[StoredProposalHunk],
    decisions: BTreeMap<String, Option<ReviewDecision>>,
) -> Result<BTreeMap<String, ReviewDecision>, WorkspaceCommandError> {
    if decisions.len() != hunks.len()
        || hunks
            .iter()
            .any(|hunk| !matches!(decisions.get(&hunk.id), Some(Some(_))))
    {
        return Err(WorkspaceCommandError::Validation(
            "Review decisions do not cover exactly every proposal hunk".into(),
        ));
    }
    decisions
        .into_iter()
        .map(|(id, decision)| {
            decision.map(|value| (id, value)).ok_or_else(|| {
                WorkspaceCommandError::Validation("Review decision is still pending".into())
            })
        })
        .collect()
}

fn review_candidate_summary(
    record: ReviewCandidateSummaryRecord,
) -> Result<ReviewCandidateSummary, WorkspaceCommandError> {
    Ok(ReviewCandidateSummary {
        schema_version: WORKSPACE_SCHEMA_VERSION,
        proposal_id: record.proposal_id,
        run_id: record.run_id,
        operation_intent_id: record.operation_intent_id,
        provider_id: record.provider_id,
        model: record.model,
        base_commit_id: record.base_commit_id,
        target_document_id: record.target_document_id,
        target_block_id: record.target_block_id,
        hunk_count: usize::try_from(record.hunk_count).map_err(|_| {
            WorkspaceCommandError::Store(StoreError::InvariantViolation(
                "review candidate hunk count is invalid".into(),
            ))
        })?,
        summary: record.summary,
        revision: record.revision,
        status: record.status.as_str().into(),
        created_at: record.created_at,
        updated_at: record.updated_at,
        candidate_branch: record.candidate_branch.map(review_candidate_branch),
    })
}

fn review_candidate_branch(record: ReviewCandidateBranchRecord) -> ReviewCandidateBranch {
    ReviewCandidateBranch {
        proposal_id: record.proposal_id,
        branch_id: record.branch_id,
        branch_name: record.branch_name,
        commit_id: record.commit_id,
        snapshot_id: record.snapshot_id,
        created_at: record.created_at,
    }
}

fn verify_stored_proposal_hash(
    payload: &Value,
    binding_hash: &str,
) -> Result<(), WorkspaceCommandError> {
    let mut canonical = payload.clone();
    let object = canonical.as_object_mut().ok_or_else(|| {
        WorkspaceCommandError::Validation("Stored proposal payload must be an object".into())
    })?;
    let declared = object
        .remove("proposalHash")
        .and_then(|value| value.as_str().map(str::to_owned))
        .ok_or_else(|| {
            WorkspaceCommandError::Validation("Stored proposal has no proposalHash".into())
        })?;
    let encoded = serde_json::to_vec(&canonical).map_err(WorkspaceCommandError::Json)?;
    let calculated = sha256(&encoded);
    if declared != binding_hash || calculated != binding_hash {
        return Err(WorkspaceCommandError::Validation(
            "Stored proposal hash verification failed".into(),
        ));
    }
    Ok(())
}

fn validate_proposal_hunks(
    source: &str,
    target: &StoredProposalTarget,
    hunks: &[StoredProposalHunk],
) -> Result<(), WorkspaceCommandError> {
    if hunks.is_empty() || hunks.len() > 500 {
        return Err(WorkspaceCommandError::Validation(
            "Stored proposal must contain 1..=500 hunks".into(),
        ));
    }
    utf16_to_byte(source, target.from.offset).ok_or_else(|| {
        WorkspaceCommandError::Validation("Proposal target start is not a UTF-16 boundary".into())
    })?;
    utf16_to_byte(source, target.to.offset).ok_or_else(|| {
        WorkspaceCommandError::Validation("Proposal target end is not a UTF-16 boundary".into())
    })?;
    let mut ids = BTreeSet::new();
    let mut previous_end = target.from.offset;
    let mut previous_insertion = None;
    for hunk in hunks {
        if hunk.id.trim().is_empty() || !ids.insert(hunk.id.as_str()) {
            return Err(WorkspaceCommandError::Validation(
                "Proposal hunk ids must be non-empty and unique".into(),
            ));
        }
        if hunk.from.block_id != target.block_id
            || hunk.to.block_id != target.block_id
            || hunk.from.offset > hunk.to.offset
            || hunk.from.offset < target.from.offset
            || hunk.to.offset > target.to.offset
            || hunk.from.offset < previous_end
            || (hunk.from.offset == hunk.to.offset && previous_insertion == Some(hunk.from.offset))
        {
            return Err(WorkspaceCommandError::Validation(
                "Proposal hunks are outside the target, ambiguous or overlapping".into(),
            ));
        }
        let from = utf16_to_byte(source, hunk.from.offset).ok_or_else(|| {
            WorkspaceCommandError::Validation("Hunk start is not a UTF-16 boundary".into())
        })?;
        let to = utf16_to_byte(source, hunk.to.offset).ok_or_else(|| {
            WorkspaceCommandError::Validation("Hunk end is not a UTF-16 boundary".into())
        })?;
        if source.get(from..to) != Some(hunk.original.as_str()) {
            let actual = source.get(from..to).unwrap_or_default();
            return Err(WorkspaceCommandError::Store(StoreError::Conflict {
                entity: "proposal_hunk",
                id: hunk.id.clone(),
                expected_revision: target.base_revision,
                actual_revision: target.base_revision,
                expected_hash: sha256(hunk.original.as_bytes()),
                actual_hash: sha256(actual.as_bytes()),
            }));
        }
        previous_end = hunk.to.offset;
        previous_insertion = (hunk.from.offset == hunk.to.offset).then_some(hunk.from.offset);
    }
    Ok(())
}

fn validate_atomic_decisions(
    hunks: &[StoredProposalHunk],
    decisions: &BTreeMap<String, ReviewDecision>,
) -> Result<(), WorkspaceCommandError> {
    let mut groups = BTreeMap::new();
    for hunk in hunks {
        let Some(group) = hunk.atomic_group.as_deref() else {
            continue;
        };
        if group.trim().is_empty() {
            return Err(WorkspaceCommandError::Validation(
                "Atomic proposal group cannot be empty".into(),
            ));
        }
        let decision = decisions[&hunk.id];
        if groups
            .insert(group, decision)
            .is_some_and(|prior| prior != decision)
        {
            return Err(WorkspaceCommandError::Validation(
                "Atomic proposal group has inconsistent decisions".into(),
            ));
        }
    }
    Ok(())
}

fn apply_proposal_hunks(
    source: &str,
    accepted: &[&StoredProposalHunk],
) -> Result<String, WorkspaceCommandError> {
    let mut output = source.to_owned();
    let mut descending = accepted.to_vec();
    descending.sort_unstable_by(|left, right| {
        right
            .from
            .offset
            .cmp(&left.from.offset)
            .then_with(|| right.to.offset.cmp(&left.to.offset))
    });
    for hunk in descending {
        let from = utf16_to_byte(&output, hunk.from.offset).ok_or_else(|| {
            WorkspaceCommandError::Validation("Hunk start is not a UTF-16 boundary".into())
        })?;
        let to = utf16_to_byte(&output, hunk.to.offset).ok_or_else(|| {
            WorkspaceCommandError::Validation("Hunk end is not a UTF-16 boundary".into())
        })?;
        output.replace_range(from..to, &hunk.replacement);
    }
    Ok(output)
}

fn utf16_to_byte(value: &str, target: usize) -> Option<usize> {
    if target == 0 {
        return Some(0);
    }
    let mut utf16 = 0;
    for (byte, character) in value.char_indices() {
        if utf16 == target {
            return Some(byte);
        }
        utf16 += character.len_utf16();
        if utf16 > target {
            return None;
        }
    }
    (utf16 == target).then_some(value.len())
}

pub(crate) fn block_content_hash(
    kind: &str,
    content: &Value,
    plain_text: &str,
    locked: bool,
) -> Result<String, WorkspaceCommandError> {
    let payload = serde_json::to_vec(&json!({
        "content": content,
        "kind": kind,
        "locked": locked,
        "plainText": plain_text,
    }))
    .map_err(WorkspaceCommandError::Json)?;
    Ok(sha256(&payload))
}

pub(crate) fn document_state_hash(document: &DocumentRecord, active: bool) -> String {
    let mut payload = b"optimizer-document-state-v1\0".to_vec();
    append_hash_field(&mut payload, document.id.as_bytes());
    match &document.parent_id {
        Some(parent_id) => {
            payload.push(1);
            append_hash_field(&mut payload, parent_id.as_bytes());
        }
        None => payload.push(0),
    }
    append_hash_field(&mut payload, document.kind.as_bytes());
    append_hash_field(&mut payload, document.title.as_bytes());
    append_hash_field(&mut payload, document.order_key.as_bytes());
    payload.push(u8::from(active));
    sha256(&payload)
}

pub(crate) fn project_root_hash<'a, 'b>(
    documents: impl IntoIterator<Item = &'a DocumentRecord>,
    blocks: impl IntoIterator<Item = (&'b str, &'b str)>,
) -> String {
    let mut entries = documents
        .into_iter()
        .map(|document| {
            (
                format!("document:{}", document.id),
                document_state_hash(document, true),
            )
        })
        .chain(
            blocks
                .into_iter()
                .map(|(id, hash)| (format!("block:{id}"), hash.to_owned())),
        )
        .collect::<Vec<_>>();
    entries.sort_unstable_by(|left, right| left.0.cmp(&right.0));
    let mut payload = b"optimizer-project-root-v2\0".to_vec();
    for (entity_id, state_hash) in entries {
        append_hash_field(&mut payload, entity_id.as_bytes());
        append_hash_field(&mut payload, state_hash.as_bytes());
    }
    sha256(&payload)
}

fn append_hash_field(payload: &mut Vec<u8>, value: &[u8]) {
    payload.extend_from_slice(value.len().to_string().as_bytes());
    payload.push(b':');
    payload.extend_from_slice(value);
}

fn save_response(
    store: &OptimizerStore,
    project_id: &str,
    receipt: EditReceipt,
) -> Result<SaveBlockResponse, WorkspaceCommandError> {
    let project = store.get_project(project_id)?;
    let block = workspace_block(store.get_project_block(project_id, &receipt.block_id)?)?;
    Ok(SaveBlockResponse {
        schema_version: WORKSPACE_SCHEMA_VERSION,
        edit_id: receipt.edit_id,
        commit_id: receipt.commit_id,
        previous_head_commit_id: receipt.previous_head_commit_id,
        head_commit_id: project.head_commit_id,
        project_revision: project.revision,
        block,
    })
}

fn workspace_document(record: DocumentRecord) -> WorkspaceDocument {
    WorkspaceDocument {
        id: record.id,
        parent_id: record.parent_id,
        kind: record.kind,
        title: record.title,
        order_key: record.order_key,
        revision: record.revision,
    }
}

fn style_sample(record: StyleSampleRecord) -> Result<StyleSample, WorkspaceCommandError> {
    if record.id.trim().is_empty()
        || record.title.trim().is_empty()
        || record.content.trim().is_empty()
        || record.revision < 0
        || record.content_hash != sha256(record.content.as_bytes())
        || !matches!(record.status.as_str(), "canonical" | "archived")
        || !matches!(
            record.sensitivity.as_str(),
            "local_sensitive" | "never_send"
        )
    {
        return Err(WorkspaceCommandError::Store(
            StoreError::InvariantViolation("stored style sample policy is invalid".into()),
        ));
    }
    Ok(StyleSample {
        schema_version: WORKSPACE_SCHEMA_VERSION,
        id: record.id,
        title: record.title,
        content: record.content,
        content_hash: record.content_hash,
        status: record.status,
        sensitivity: record.sensitivity,
        revision: record.revision,
        created_at: record.created_at,
        updated_at: record.updated_at,
    })
}

fn knowledge_item(record: KnowledgeItemRecord) -> Result<KnowledgeItem, WorkspaceCommandError> {
    validate_stored_knowledge_item(&record)?;
    Ok(KnowledgeItem {
        schema_version: WORKSPACE_SCHEMA_VERSION,
        id: record.id,
        kind: record.kind,
        title: record.title,
        content: record.content,
        content_hash: record.content_hash,
        status: record.status,
        authority: record.authority,
        sensitivity: record.sensitivity,
        severity: record.severity,
        revision: record.revision,
        created_at: record.created_at,
        updated_at: record.updated_at,
    })
}

fn knowledge_context_candidate(
    record: KnowledgeItemRecord,
    source_commit_id: &str,
) -> Result<KnowledgeContextCandidate, WorkspaceCommandError> {
    validate_stored_knowledge_item(&record)?;
    let label = match (record.kind.as_str(), record.severity.as_deref()) {
        ("fact", None) => "事实",
        ("constraint", Some("hard")) => "硬约束",
        ("constraint", Some("soft")) => "软约束",
        _ => {
            return Err(WorkspaceCommandError::Store(
                StoreError::InvariantViolation("stored knowledge type is invalid".into()),
            ));
        }
    };
    let content = format!("{label}【{}】：{}", record.title, record.content);
    let reason_code = match (record.kind.as_str(), record.severity.as_deref()) {
        ("fact", None) => "CANONICAL_FACT",
        ("constraint", Some("hard")) => "PROJECT_HARD_CONSTRAINT",
        ("constraint", Some("soft")) => "PROJECT_SOFT_CONSTRAINT",
        _ => {
            return Err(invariant_violation(
                "stored knowledge policy failed validation",
            ));
        }
    };
    Ok(KnowledgeContextCandidate {
        id: format!("knowledge-{}-r{}", record.id, record.revision),
        source_ref: format!(
            "knowledge:{}:{}@r{}",
            record.kind, record.id, record.revision
        ),
        source_hash: sha256(content.as_bytes()),
        source_commit_id: source_commit_id.to_owned(),
        tier: "L3_KNOWLEDGE".into(),
        status: record.status,
        authority: record.authority,
        sensitivity: record.sensitivity,
        render_mode: "constraint".into(),
        reason_codes: vec![reason_code.into()],
        content,
        revision: record.revision,
        generated_at: record.updated_at,
    })
}

fn checkpoint_summary(record: SnapshotRecord) -> CheckpointSummary {
    CheckpointSummary {
        id: record.id,
        commit_id: record.commit_id,
        root_hash: record.root_hash,
        codec: record.codec,
        codec_version: record.codec_version,
        checksum: record.checksum,
        created_at: record.created_at,
    }
}

fn version_commit(record: CommitRecord) -> VersionCommit {
    VersionCommit {
        id: record.id,
        root_hash: record.root_hash,
        reason: record.reason,
        actor_type: record.actor_type,
        actor_id: record.actor_id,
        created_at: record.created_at,
        parents: record.parents,
    }
}

fn workspace_block(record: BlockRecord) -> Result<WorkspaceBlock, WorkspaceCommandError> {
    let content = serde_json::from_str(&record.content_json).map_err(|source| {
        WorkspaceCommandError::StoredContent {
            block_id: record.id.clone(),
            source,
        }
    })?;
    Ok(WorkspaceBlock {
        id: record.id,
        document_id: record.document_id,
        kind: record.kind,
        order_key: record.order_key,
        content,
        plain_text: record.plain_text,
        content_hash: record.content_hash,
        revision: record.revision,
        locked: record.locked,
    })
}

fn validate_save_spec(spec: &SaveBlockSpec) -> Result<(), WorkspaceCommandError> {
    if spec.block_id.trim().is_empty()
        || spec.block_id.len() > 200
        || spec.expected_hash.trim().is_empty()
        || spec.expected_revision < 0
    {
        return Err(WorkspaceCommandError::Validation(
            "Block id, expected revision and expected hash are invalid".into(),
        ));
    }
    if !spec.content.is_object() {
        return Err(WorkspaceCommandError::Validation(
            "Block content must be a JSON object".into(),
        ));
    }
    if spec.plain_text.len() > MAX_PLAIN_TEXT_BYTES {
        return Err(WorkspaceCommandError::Validation(format!(
            "Block plain text exceeds {MAX_PLAIN_TEXT_BYTES} bytes"
        )));
    }
    Ok(())
}

fn validate_style_sample(spec: &CreateStyleSampleSpec) -> Result<(), WorkspaceCommandError> {
    let title = spec.title.trim();
    let content = spec.content.trim();
    if title.is_empty() || title.chars().count() > MAX_STYLE_TITLE_CHARS {
        return Err(WorkspaceCommandError::StyleValidation(format!(
            "Style sample title must contain 1 to {MAX_STYLE_TITLE_CHARS} characters"
        )));
    }
    if content.is_empty() || content.len() > MAX_STYLE_SAMPLE_BYTES {
        return Err(WorkspaceCommandError::StyleValidation(format!(
            "Style sample content must contain 1 to {MAX_STYLE_SAMPLE_BYTES} bytes"
        )));
    }
    if !matches!(spec.sensitivity.as_str(), "local_sensitive" | "never_send") {
        return Err(WorkspaceCommandError::StyleValidation(
            "Style sample sensitivity must be local_sensitive or never_send".into(),
        ));
    }
    Ok(())
}

fn validate_knowledge_item(spec: &CreateKnowledgeItemSpec) -> Result<(), WorkspaceCommandError> {
    let title = spec.title.trim();
    let content = spec.content.trim();
    if title.is_empty() || title.chars().count() > MAX_KNOWLEDGE_TITLE_CHARS {
        return Err(WorkspaceCommandError::KnowledgeValidation(format!(
            "Knowledge title must contain 1 to {MAX_KNOWLEDGE_TITLE_CHARS} characters"
        )));
    }
    if content.is_empty() || content.len() > MAX_KNOWLEDGE_CONTENT_BYTES {
        return Err(WorkspaceCommandError::KnowledgeValidation(format!(
            "Knowledge content must contain 1 to {MAX_KNOWLEDGE_CONTENT_BYTES} bytes"
        )));
    }
    if !matches!(spec.kind.as_str(), "fact" | "constraint") {
        return Err(WorkspaceCommandError::KnowledgeValidation(
            "Knowledge kind must be fact or constraint".into(),
        ));
    }
    let valid_severity = match spec.kind.as_str() {
        "fact" => spec.severity.is_none(),
        "constraint" => matches!(spec.severity.as_deref(), Some("hard" | "soft")),
        _ => false,
    };
    if !valid_severity {
        return Err(WorkspaceCommandError::KnowledgeValidation(
            "Facts have no severity; constraints require hard or soft severity".into(),
        ));
    }
    if !matches!(
        spec.sensitivity.as_str(),
        "public" | "local" | "local_sensitive" | "never_send"
    ) {
        return Err(WorkspaceCommandError::KnowledgeValidation(
            "Knowledge sensitivity is unsupported".into(),
        ));
    }
    Ok(())
}

fn validate_stored_knowledge_item(
    record: &KnowledgeItemRecord,
) -> Result<(), WorkspaceCommandError> {
    let identity_valid = !record.id.trim().is_empty()
        && !record.project_id.trim().is_empty()
        && !record.title.trim().is_empty()
        && !record.content.trim().is_empty()
        && !record.created_at.trim().is_empty()
        && !record.updated_at.trim().is_empty()
        && record.revision >= 0;
    let hash_valid = record.content_hash == sha256(record.content.as_bytes());
    let status_valid = matches!(
        record.status.as_str(),
        "canonical" | "archived" | "rejected"
    );
    let authority_valid = matches!(
        record.authority.as_str(),
        "user_confirmed" | "source_derived" | "model_inferred" | "external_untrusted"
    );
    let sensitivity_valid = matches!(
        record.sensitivity.as_str(),
        "public" | "local" | "local_sensitive" | "never_send"
    );
    let type_valid = match record.kind.as_str() {
        "fact" => record.severity.is_none(),
        "constraint" => matches!(record.severity.as_deref(), Some("hard" | "soft")),
        _ => false,
    };
    if !identity_valid
        || !hash_valid
        || !status_valid
        || !authority_valid
        || !sensitivity_valid
        || !type_valid
    {
        return Err(WorkspaceCommandError::Store(
            StoreError::InvariantViolation("stored knowledge policy is invalid".into()),
        ));
    }
    Ok(())
}

fn now() -> Result<String, WorkspaceCommandError> {
    OffsetDateTime::now_utc()
        .format(&Rfc3339)
        .map_err(|_| WorkspaceCommandError::Clock)
}

fn generated_id(prefix: &str) -> String {
    format!("{prefix}-{}", Uuid::new_v4().simple())
}

fn sha256(payload: &[u8]) -> String {
    let mut encoded = String::with_capacity(71);
    encoded.push_str("sha256:");
    for byte in Sha256::digest(payload) {
        let _ = write!(encoded, "{byte:02x}");
    }
    encoded
}

// ---------------------------------------------------------------------------
// v0.8.0 Stage 1 (FR-11) — project-level backup & restore
// ---------------------------------------------------------------------------

const BACKUP_MANIFEST_SCHEMA_VERSION: i64 = 1;
const BACKUP_OPTIMIZER_VERSION: &str = "0.8.0";
const BACKUP_ARCHIVE_EXTENSION: &str = ".optimizer-backup";
const BACKUP_PROJECT_PREFIX: &str = "project/";
const BACKUP_MANIFEST_FILE: &str = "manifest.json";
const BACKUP_ENDPOINTS_FILE: &str = "endpoints.json";
const BACKUP_RECENT_FILE: &str = "recent.json";
const BACKUP_DATABASE_FILE: &str = "project.sqlite3";
const BACKUP_MAX_ARCHIVE_BYTES: u64 = 2 * 1024 * 1024 * 1024; // 2 GiB safety cap
const BACKUP_MAX_EXTRACTED_BYTES: u64 = 4 * 1024 * 1024 * 1024; // 4 GiB zip-bomb guard

/// Request to export an entire `.optimizer` project package as a single
/// `.optimizer-backup` zip archive. The caller (Tauri command layer) is
/// responsible for providing endpoint metadata and recent-project metadata
/// as JSON strings — the host core never touches the Secret Store.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct ExportProjectBackupRequest {
    pub project_id: String,
    pub include_endpoints: bool,
    pub include_recent: bool,
    pub output_path: String,
    /// Serialized endpoint metadata (no secrets). When `include_endpoints`
    /// is `true` but this is `None`, an empty array `"[]"` is written.
    pub endpoints_json: Option<String>,
    /// Serialized recent-project metadata for this project. When
    /// `include_recent` is `true` but this is `None`, the field is omitted
    /// from the archive.
    pub recent_json: Option<String>,
}

#[derive(Debug, Clone, PartialEq, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct ExportProjectBackupResponse {
    pub schema_version: u32,
    pub archive_path: String,
    pub manifest: BackupManifest,
    pub bytes_written: u64,
    pub item_count: usize,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct ImportProjectBackupRequest {
    pub archive_path: String,
    pub target_directory: String,
    pub new_project_id: Option<String>,
    pub overwrite: bool,
}

#[derive(Debug, Clone, PartialEq, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct ImportProjectBackupResponse {
    pub schema_version: u32,
    pub project_id: String,
    pub project_path: String,
    pub manifest: BackupManifest,
    pub restored_items: Vec<String>,
    pub warnings: Vec<String>,
    /// Always `"backup"` for restores from a `.optimizer-backup` archive.
    /// The frontend uses this to badge the project in the recent-projects
    /// list (contract T09).
    pub source: String,
}

/// Walk the project package root and add every file to the zip writer
/// under the `project/` prefix. Returns the number of files added.
fn add_directory_to_zip<W: Write + Seek>(
    zip: &mut zip::ZipWriter<W>,
    options: zip::write::SimpleFileOptions,
    root: &Path,
    prefix: &str,
    excluded_root_dir_names: &[&str],
) -> Result<usize, WorkspaceCommandError> {
    let mut count = 0usize;
    let mut stack = vec![std::path::PathBuf::from("")];
    while let Some(relative) = stack.pop() {
        let absolute = root.join(&relative);
        let entries = std::fs::read_dir(&absolute).map_err(|source| {
            WorkspaceCommandError::Validation(format!(
                "failed to read directory {}: {source}",
                absolute.display()
            ))
        })?;
        for entry in entries.flatten() {
            let entry_path = entry.path();
            let entry_relative = relative.join(entry.file_name());
            let zip_name = format!("{prefix}/{}", entry_relative.to_string_lossy());
            let metadata = entry.metadata().map_err(|source| {
                WorkspaceCommandError::Validation(format!(
                    "failed to inspect {}: {source}",
                    entry_path.display()
                ))
            })?;
            if metadata.is_dir() {
                // Skip top-level directories that must never be bundled
                // (e.g. the migration `backups/` folder, which can be very
                // large and is fully rebuildable).
                if relative.as_os_str().is_empty()
                    && entry
                        .file_name()
                        .to_str()
                        .is_some_and(|name| excluded_root_dir_names.contains(&name))
                {
                    continue;
                }
                zip.add_directory(&zip_name, options)
                    .map_err(|source| WorkspaceCommandError::Validation(
                        format!("failed to add directory to archive: {source}"),
                    ))?;
                stack.push(entry_relative);
                count += 1;
            } else if metadata.is_file() {
                let file_bytes = std::fs::read(&entry_path).map_err(|source| {
                    WorkspaceCommandError::Validation(format!(
                        "failed to read file {}: {source}",
                        entry_path.display()
                    ))
                })?;
                zip.start_file(&zip_name, options)
                    .map_err(|source| WorkspaceCommandError::Validation(
                        format!("failed to start file in archive: {source}"),
                    ))?;
                zip.write_all(&file_bytes).map_err(|source| {
                    WorkspaceCommandError::Validation(format!(
                        "failed to write file to archive: {source}"
                    ))
                })?;
                count += 1;
            }
        }
    }
    Ok(count)
}

pub(crate) fn export_project_backup(
    store: &OptimizerStore,
    project_root: &Path,
    request: &ExportProjectBackupRequest,
) -> Result<ExportProjectBackupResponse, WorkspaceCommandError> {
    if request.project_id.trim().is_empty() {
        return Err(WorkspaceCommandError::Validation(
            "project_id must be a non-empty string".into(),
        ));
    }
    if request.output_path.trim().is_empty() {
        return Err(WorkspaceCommandError::Validation(
            "output_path must be a non-empty string".into(),
        ));
    }

    let output_path = Path::new(&request.output_path);
    if !output_path.is_absolute() {
        return Err(WorkspaceCommandError::Validation(
            "output_path must be absolute".into(),
        ));
    }

    // Ensure the archive ends with `.optimizer-backup`.
    let final_output = if output_path
        .extension()
        .and_then(|ext| ext.to_str())
        .map(|ext| ext == "optimizer-backup")
        .unwrap_or(false)
    {
        output_path.to_path_buf()
    } else {
        let mut path = output_path.as_os_str().to_os_string();
        path.push(BACKUP_ARCHIVE_EXTENSION);
        PathBuf::from(path)
    };

    // Create parent directory if needed.
    if let Some(parent) = final_output.parent()
        && !parent.as_os_str().is_empty()
        && !parent.exists()
    {
        std::fs::create_dir_all(parent).map_err(|source| {
            WorkspaceCommandError::Validation(format!(
                "failed to create output parent directory: {source}"
            ))
        })?;
    }

    let generated_at = now()?;
    let db_version = store
        .diagnostics()
        .map_err(WorkspaceCommandError::from)?
        .schema_version;

    let mut included_items = vec![
        "project.sqlite3".to_string(),
        "manifest.json".to_string(),
        "assets".to_string(),
        // `backups/` (pre-migration snapshots) is deliberately excluded: it
        // can be very large and is rebuildable. `exports/` remains included.
        "exports".to_string(),
    ];
    if request.include_endpoints {
        included_items.push("endpoints.json".to_string());
    }
    if request.include_recent {
        included_items.push("recent.json".to_string());
    }

    let manifest = BackupManifest {
        schema_version: BACKUP_MANIFEST_SCHEMA_VERSION,
        source_project_id: request.project_id.clone(),
        source_path: project_root.to_string_lossy().into_owned(),
        generated_at: generated_at.clone(),
        included_items: included_items.clone(),
        optimizer_version: BACKUP_OPTIMIZER_VERSION.into(),
        schema_db_version: db_version,
    };

    let manifest_json = serde_json::to_vec_pretty(&manifest)
        .map_err(|source| WorkspaceCommandError::Validation(
            format!("failed to serialize backup manifest: {source}"),
        ))?;

    let endpoints_bytes = if request.include_endpoints {
        Some(
            request
                .endpoints_json
                .clone()
                .unwrap_or_else(|| "[]".to_string())
                .into_bytes(),
        )
    } else {
        None
    };

    let recent_bytes = if request.include_recent {
        request.recent_json.clone().map(|s| s.into_bytes())
    } else {
        None
    };

    // Write to a temporary file first, then rename for atomicity.
    let temp_path = final_output.with_extension("optimizer-backup.tmp");
    if temp_path.exists() {
        let _ = std::fs::remove_file(&temp_path);
    }

    let file = std::fs::File::create(&temp_path).map_err(|source| {
        WorkspaceCommandError::Validation(format!(
            "failed to create temporary archive file: {source}"
        ))
    })?;
    let mut zip = zip::ZipWriter::new(file);
    let options = zip::write::SimpleFileOptions::default()
        .compression_method(zip::CompressionMethod::Deflated);

    // Write manifest.json at the archive root.
    zip.start_file(BACKUP_MANIFEST_FILE, options)
        .map_err(|source| WorkspaceCommandError::Validation(
            format!("failed to start manifest in archive: {source}"),
        ))?;
    zip.write_all(&manifest_json)
        .map_err(|source| WorkspaceCommandError::Validation(
            format!("failed to write manifest to archive: {source}"),
        ))?;

    // Optional endpoints.json at the archive root.
    if let Some(ep_bytes) = &endpoints_bytes {
        zip.start_file(BACKUP_ENDPOINTS_FILE, options)
            .map_err(|source| WorkspaceCommandError::Validation(
                format!("failed to start endpoints in archive: {source}"),
            ))?;
        zip.write_all(ep_bytes)
            .map_err(|source| WorkspaceCommandError::Validation(
                format!("failed to write endpoints to archive: {source}"),
            ))?;
    }

    // Optional recent.json at the archive root.
    if let Some(recent) = &recent_bytes {
        zip.start_file(BACKUP_RECENT_FILE, options)
            .map_err(|source| WorkspaceCommandError::Validation(
                format!("failed to start recent metadata in archive: {source}"),
            ))?;
        zip.write_all(recent)
            .map_err(|source| WorkspaceCommandError::Validation(
                format!("failed to write recent metadata to archive: {source}"),
            ))?;
    }

    // Add the entire project directory under `project/`, excluding the
    // rebuildable migration `backups/` folder.
    let file_count =
        add_directory_to_zip(&mut zip, options, project_root, "project", &["backups"])?;

    zip.finish()
        .map_err(|source| WorkspaceCommandError::Validation(
            format!("failed to finalize backup archive: {source}"),
        ))?;

    let bytes_written = std::fs::metadata(&temp_path)
        .map(|m| m.len())
        .unwrap_or(0);
    if bytes_written > BACKUP_MAX_ARCHIVE_BYTES {
        let _ = std::fs::remove_file(&temp_path);
        return Err(WorkspaceCommandError::Validation(
            "backup archive exceeds maximum size".into(),
        ));
    }

    // Atomic rename.
    if final_output.exists() {
        let _ = std::fs::remove_file(&final_output);
    }
    std::fs::rename(&temp_path, &final_output).map_err(|source| {
        let _ = std::fs::remove_file(&temp_path);
        WorkspaceCommandError::Validation(format!(
            "failed to publish backup archive: {source}"
        ))
    })?;

    Ok(ExportProjectBackupResponse {
        schema_version: 1,
        archive_path: final_output.to_string_lossy().into_owned(),
        manifest,
        bytes_written,
        item_count: file_count,
    })
}

/// Guard against zip-slip: ensure `entry_name` normalizes to a path inside
/// `target` with no `..` traversal or absolute escape.
fn safe_extract_path(target: &Path, entry_name: &str) -> Option<PathBuf> {
    // Normalize backslashes (Windows archives may use them).
    let normalized = entry_name.replace('\\', "/");
    let path = Path::new(&normalized);
    if path.is_absolute() {
        return None;
    }
    let mut result = target.to_path_buf();
    for component in path.components() {
        match component {
            std::path::Component::Normal(part) => result.push(part),
            std::path::Component::CurDir => {}
            _ => return None, // reject ParentDir, RootDir, Prefix
        }
    }
    // Final check: the resolved path must be inside target.
    if result.starts_with(target) {
        Some(result)
    } else {
        None
    }
}

/// Determine the package folder name from the backup manifest's `source_path`.
/// Falls back to `Restored.optimizer` if the source path has no usable name.
fn package_folder_name_from_manifest(manifest: &BackupManifest) -> String {
    let path = Path::new(&manifest.source_path);
    if let Some(name) = path.file_name().and_then(|n| n.to_str())
        && !name.is_empty()
        && name.ends_with(".optimizer")
    {
        return name.to_string();
    }
    // Fall back to the directory name + .optimizer suffix.
    if let Some(parent) = path.file_name().and_then(|n| n.to_str())
        && !parent.is_empty()
    {
        return format!("{parent}.optimizer");
    }
    "Restored.optimizer".to_string()
}

/// Quote a SQL identifier for use inside double quotes, escaping any
/// embedded double quotes per the SQLite identifier rule (`"` -> `""`).
///
/// The names fed to this helper come from `sqlite_master` in the *archived*
/// database, which is attacker-influenced, so they must never be spliced
/// into SQL verbatim.
fn quote_sql_identifier(identifier: &str) -> String {
    format!("\"{}\"", identifier.replace('"', "\"\""))
}

/// Quote a SQL string literal inside single quotes, escaping any embedded
/// single quotes per the SQLite literal rule (`'` -> `''`).
fn quote_sql_string_literal(value: &str) -> String {
    format!("'{}'", value.replace('\'', "''"))
}

/// Mirrors the IPC-side `is_safe_binding_id` guard: an id must be 1..=128
/// ASCII alphanumeric characters with only `_` / `-` separators, so it can be
/// safely embedded in SQL identifiers and filesystem paths.
fn is_safe_binding_id(value: &str) -> bool {
    !value.is_empty()
        && value.len() <= 128
        && value
            .bytes()
            .all(|byte| byte.is_ascii_alphanumeric() || matches!(byte, b'_' | b'-'))
}

/// Remap `project_id` across all SQLite tables that carry the column.
/// Uses a dynamic schema query to stay resilient to future migrations.
///
/// Two obstacles must be overcome:
///
/// 1. **Foreign-key constraints** — `project.id` is the PK parent for 13+
///    child tables. Updating the parent row first would trip
///    `FOREIGN KEY constraint failed` while children still reference the
///    old id. We use `PRAGMA defer_foreign_keys = ON` inside a transaction
///    so FK checks are deferred until `COMMIT`, at which point every table
///    has been updated.
///
/// 2. **Immutable triggers** — Tables like `commit_node`, `edit_journal`,
///    `change_set`, etc. carry `BEFORE UPDATE` triggers that
///    `RAISE(ABORT, 'immutable:<table>')` on any UPDATE. We temporarily
///    drop these triggers (saving their SQL), perform the remap, and
///    recreate them before `COMMIT` so the schema is fully restored.
///
/// The whole remap runs inside a single rusqlite `Transaction` opened with
/// `TransactionBehavior::Immediate`; on any error the transaction is
/// rolled back (including on early `?` returns via `Drop`).
fn remap_project_id(
    connection: &mut rusqlite::Connection,
    old_id: &str,
    new_id: &str,
) -> Result<usize, WorkspaceCommandError> {
    // Collect all immutable trigger definitions so we can restore them.
    let immutable_triggers: Vec<(String, String)> = {
        let mut stmt = connection
            .prepare(
                "SELECT name, sql FROM sqlite_master \
                 WHERE type = 'trigger' AND name LIKE '%_immutable_%'",
            )
            .map_err(|source| WorkspaceCommandError::Store(StoreError::Sqlite(source)))?;
        let rows = stmt
            .query_map([], |row| {
                Ok((row.get::<_, String>(0)?, row.get::<_, String>(1)?))
            })
            .map_err(|source| WorkspaceCommandError::Store(StoreError::Sqlite(source)))?;
        rows.filter_map(|r| r.ok()).collect()
    };

    let transaction = connection
        .transaction_with_behavior(rusqlite::TransactionBehavior::Immediate)
        .map_err(|source| WorkspaceCommandError::Store(StoreError::Sqlite(source)))?;

    // Explicitly enable deferred foreign key checks inside this transaction,
    // then verify the pragma actually took effect before mutating data.
    transaction
        .execute_batch("PRAGMA defer_foreign_keys = ON")
        .map_err(|source| WorkspaceCommandError::Store(StoreError::Sqlite(source)))?;
    let defer_enabled: i64 = transaction
        .query_row("PRAGMA defer_foreign_keys", [], |row| row.get(0))
        .map_err(|source| WorkspaceCommandError::Store(StoreError::Sqlite(source)))?;
    if defer_enabled != 1 {
        return Err(WorkspaceCommandError::Validation(
            "deferred foreign key checks could not be enabled for project remap".into(),
        ));
    }

    // Temporarily drop all immutable triggers so we can UPDATE the
    // immutable tables without tripping RAISE(ABORT). Identifiers are
    // double-quoted and escaped (they originate from the archived DB).
    for (name, _) in &immutable_triggers {
        let sql = format!(
            "DROP TRIGGER IF EXISTS {}",
            quote_sql_identifier(name)
        );
        transaction
            .execute_batch(&sql)
            .map_err(|source| WorkspaceCommandError::Store(StoreError::Sqlite(source)))?;
    }

    // Update the canonical `project` row.
    let updated = transaction
        .execute(
            "UPDATE project SET id = ?1 WHERE id = ?2",
            rusqlite::params![new_id, old_id],
        )
        .map_err(|source| WorkspaceCommandError::Store(StoreError::Sqlite(source)))?;
    if updated == 0 {
        return Err(WorkspaceCommandError::Store(StoreError::NotFound {
            entity: "project",
            id: old_id.to_string(),
        }));
    }

    // Find all tables (except `project`) that have a `project_id` column.
    // Scoped in a block so the borrowed `Statement`/rows are dropped before
    // `transaction.commit()` consumes the transaction below.
    let tables: Vec<String> = {
        let mut tables: Vec<String> = Vec::new();
        let mut stmt = transaction
            .prepare(
                "SELECT name FROM sqlite_master WHERE type = 'table' AND name NOT IN ('project','schema_migration')",
            )
            .map_err(|source| {
                WorkspaceCommandError::Validation(format!(
                    "failed to query table list: {source}"
                ))
            })?;
        let table_rows = stmt
            .query_map([], |row| row.get::<_, String>(0))
            .map_err(|source| {
                WorkspaceCommandError::Validation(format!(
                    "failed to read table list: {source}"
                ))
            })?;
        for table_name in table_rows.flatten() {
            // Check if this table has a `project_id` column. `pragma_table_info`
            // takes a *string literal* argument (not an identifier), so it is
            // single-quoted and escaped.
            let sql = format!(
                "SELECT COUNT(*) > 0 FROM pragma_table_info({}) WHERE name = 'project_id'",
                quote_sql_string_literal(&table_name)
            );
            let has_project_id: bool = transaction
                .query_row(&sql, [], |row| row.get(0))
                .map_err(|source| WorkspaceCommandError::Store(StoreError::Sqlite(source)))?;
            if has_project_id {
                tables.push(table_name);
            }
        }
        tables
    };

    let mut total = updated;
    for table in tables {
        let sql = format!(
            "UPDATE {} SET project_id = ?1 WHERE project_id = ?2",
            quote_sql_identifier(&table)
        );
        let changed = transaction
            .execute(&sql, rusqlite::params![new_id, old_id])
            .map_err(|source| WorkspaceCommandError::Store(StoreError::Sqlite(source)))?;
        total += changed;
    }

    // Special case: summary_invalidation.scope_id stores the project_id
    // verbatim when scope_type = 'project'. The generic loop above only
    // updates columns named 'project_id', so scope_id is left stale and
    // the verify_invariants() check
    //   `si.scope_type = 'project' AND si.scope_id <> si.project_id`
    // would fail.
    let scope_changed = transaction
        .execute(
            "UPDATE summary_invalidation SET scope_id = ?1 \
             WHERE scope_type = 'project' AND scope_id = ?2",
            rusqlite::params![new_id, old_id],
        )
        .map_err(|source| WorkspaceCommandError::Store(StoreError::Sqlite(source)))?;
    total += scope_changed;

    // Recreate all immutable triggers before COMMIT so the schema is
    // fully restored.
    for (_, sql) in &immutable_triggers {
        transaction
            .execute_batch(sql)
            .map_err(|source| WorkspaceCommandError::Store(StoreError::Sqlite(source)))?;
    }

    // Commit — deferred FK constraints are checked here. If any child
    // table still references a non-existent project, this will fail.
    transaction
        .commit()
        .map_err(|source| WorkspaceCommandError::Store(StoreError::Sqlite(source)))?;

    Ok(total)
}

pub(crate) fn import_project_backup(
    request: &ImportProjectBackupRequest,
) -> Result<ImportProjectBackupResponse, WorkspaceCommandError> {
    if request.archive_path.trim().is_empty() {
        return Err(WorkspaceCommandError::Validation(
            "archive_path must be a non-empty string".into(),
        ));
    }
    if request.target_directory.trim().is_empty() {
        return Err(WorkspaceCommandError::Validation(
            "target_directory must be a non-empty string".into(),
        ));
    }

    let archive_path = Path::new(&request.archive_path);
    if !archive_path.is_absolute() {
        return Err(WorkspaceCommandError::Validation(
            "archive_path must be absolute".into(),
        ));
    }
    if !archive_path.is_file() {
        return Err(WorkspaceCommandError::Validation(
            "archive_path does not point to an existing file".into(),
        ));
    }

    let target_parent = Path::new(&request.target_directory);
    if !target_parent.is_absolute() {
        return Err(WorkspaceCommandError::Validation(
            "target_directory must be absolute".into(),
        ));
    }
    if !target_parent.is_dir() {
        return Err(WorkspaceCommandError::Validation(
            "target_directory does not point to an existing directory".into(),
        ));
    }

    // Validate new_project_id if provided.
    if let Some(ref new_id) = request.new_project_id
        && !is_safe_binding_id(new_id)
    {
        return Err(WorkspaceCommandError::Validation(
            "new_project_id must be 1..=128 ASCII letters, digits, '_' or '-'".into(),
        ));
    }

    // Open the zip archive.
    let file = std::fs::File::open(archive_path).map_err(|source| {
        WorkspaceCommandError::Validation(format!("failed to open backup archive: {source}"))
    })?;
    let mut zip = zip::ZipArchive::new(file).map_err(|source| {
        WorkspaceCommandError::Validation(format!("failed to read backup archive: {source}"))
    })?;

    // Read manifest.json from the archive root. The `ZipFile` borrow of `zip`
    // is scoped so it is released before the entries are enumerated below.
    let manifest_bytes: Vec<u8> = {
        let mut entry = zip
            .by_name(BACKUP_MANIFEST_FILE)
            .map_err(|_| WorkspaceCommandError::Validation(
                "backup archive is missing manifest.json at root".into(),
            ))?;
        let mut buffer = Vec::new();
        entry.read_to_end(&mut buffer).map_err(|source| {
            WorkspaceCommandError::Validation(format!(
                "failed to read manifest.json from archive: {source}"
            ))
        })?;
        buffer
    };
    let manifest: BackupManifest = serde_json::from_slice(&manifest_bytes).map_err(|source| {
        WorkspaceCommandError::Validation(format!(
            "failed to parse backup manifest: {source}"
        ))
    })?;

    // Validate schema_version (T07).
    if manifest.schema_version != BACKUP_MANIFEST_SCHEMA_VERSION {
        return Err(WorkspaceCommandError::Validation(format!(
            "backup manifest schema_version {} is not supported (expected {})",
            manifest.schema_version, BACKUP_MANIFEST_SCHEMA_VERSION
        )));
    }

    // Determine the package folder name and final path.
    let package_name = package_folder_name_from_manifest(&manifest);
    let final_path = target_parent.join(&package_name);

    // Check overwrite (T08).
    if final_path.exists() && !request.overwrite {
        return Err(WorkspaceCommandError::Validation(format!(
            "target package already exists: {} (set overwrite=true to replace)",
            final_path.display()
        )));
    }

    // Create a staging directory in the target parent.
    let staging_name = format!(".{package_name}.restoring-{}", Uuid::new_v4().simple());
    let staging_path = target_parent.join(&staging_name);
    std::fs::create_dir_all(&staging_path).map_err(|source| {
        WorkspaceCommandError::Validation(format!(
            "failed to create staging directory: {source}"
        ))
    })?;

    // Extract all entries from the archive.
    let mut extracted_bytes: u64 = 0;
    let mut restored_items: Vec<String> = Vec::new();
    let mut has_database = false;

    for index in 0..zip.len() {
        let mut entry = zip
            .by_index(index)
            .map_err(|source| WorkspaceCommandError::Validation(
                format!("failed to read archive entry: {source}"),
            ))?;
        let entry_name = entry.name().to_string();

        // Only extract entries under the `project/` prefix (or the manifest).
        if !entry_name.starts_with(BACKUP_PROJECT_PREFIX) {
            continue;
        }

        let relative = &entry_name[BACKUP_PROJECT_PREFIX.len()..];
        let target_path = match safe_extract_path(&staging_path, relative) {
            Some(p) => p,
            None => {
                let _ = std::fs::remove_dir_all(&staging_path);
                return Err(WorkspaceCommandError::Validation(format!(
                    "archive contains unsafe path: {entry_name}"
                )));
            }
        };

        if entry.is_dir() {
            std::fs::create_dir_all(&target_path).map_err(|source| {
                let _ = std::fs::remove_dir_all(&staging_path);
                WorkspaceCommandError::Validation(format!(
                    "failed to create directory {}: {source}",
                    target_path.display()
                ))
            })?;
            continue;
        }

        // Ensure parent directory exists.
        if let Some(parent) = target_path.parent() {
            std::fs::create_dir_all(parent).map_err(|source| {
                let _ = std::fs::remove_dir_all(&staging_path);
                WorkspaceCommandError::Validation(format!(
                    "failed to create parent directory: {source}"
                ))
            })?;
        }

        // Zip-bomb guard: track total extracted bytes.
        let file_size = entry.size();
        extracted_bytes = extracted_bytes.saturating_add(file_size);
        if extracted_bytes > BACKUP_MAX_EXTRACTED_BYTES {
            let _ = std::fs::remove_dir_all(&staging_path);
            return Err(WorkspaceCommandError::Validation(
                "backup archive exceeds maximum extracted size".into(),
            ));
        }

        let mut buffer = Vec::with_capacity(file_size as usize);
        entry.read_to_end(&mut buffer).map_err(|source| {
            let _ = std::fs::remove_dir_all(&staging_path);
            WorkspaceCommandError::Validation(format!(
                "failed to read file from archive: {source}"
            ))
        })?;
        std::fs::write(&target_path, &buffer).map_err(|source| {
            let _ = std::fs::remove_dir_all(&staging_path);
            WorkspaceCommandError::Validation(format!(
                "failed to write extracted file: {source}"
            ))
        })?;

        let file_name = target_path
            .file_name()
            .and_then(|n| n.to_str())
            .unwrap_or("");
        if file_name == BACKUP_DATABASE_FILE {
            has_database = true;
            restored_items.push("project.sqlite3".to_string());
        } else if file_name == "manifest.json" {
            restored_items.push("manifest.json".to_string());
        }
    }

    if !has_database {
        let _ = std::fs::remove_dir_all(&staging_path);
        return Err(WorkspaceCommandError::Validation(
            "backup archive does not contain a project database".into(),
        ));
    }

    // Validate SQLite invariants (T05).
    let database_path = staging_path.join(BACKUP_DATABASE_FILE);
    let mut store = OptimizerStore::open(&database_path)
        .map_err(|source| {
            let _ = std::fs::remove_dir_all(&staging_path);
            WorkspaceCommandError::from(source)
        })?;
    store
        .verify_invariants()
        .map_err(|source| {
            let _ = std::fs::remove_dir_all(&staging_path);
            WorkspaceCommandError::from(source)
        })?;

    // Determine the final project_id (T06).
    let original_project_id = manifest.source_project_id.clone();
    let new_project_id = request
        .new_project_id
        .clone()
        .unwrap_or_else(|| format!("project-{}", Uuid::new_v4().simple()));

    let mut warnings = Vec::new();
    if new_project_id != original_project_id {
        // Remap project_id in all SQLite tables.
        remap_project_id(store.connection_mut(), &original_project_id, &new_project_id)
            .inspect_err(|_| {
                let _ = std::fs::remove_dir_all(&staging_path);
            })?;

        // Update the project package manifest.json with the new project_id.
        let manifest_path = staging_path.join("manifest.json");
        let manifest_bytes = std::fs::read(&manifest_path).map_err(|source| {
            let _ = std::fs::remove_dir_all(&staging_path);
            WorkspaceCommandError::Validation(format!(
                "failed to read project manifest: {source}"
            ))
        })?;
        let mut project_manifest: serde_json::Value =
            serde_json::from_slice(&manifest_bytes).map_err(|source| {
                let _ = std::fs::remove_dir_all(&staging_path);
                WorkspaceCommandError::Validation(format!(
                    "failed to parse project manifest: {source}"
                ))
            })?;
        project_manifest["projectId"] = serde_json::json!(new_project_id);
        let updated_bytes = serde_json::to_vec_pretty(&project_manifest).map_err(|source| {
            let _ = std::fs::remove_dir_all(&staging_path);
            WorkspaceCommandError::Validation(format!(
                "failed to serialize updated manifest: {source}"
            ))
        })?;
        std::fs::write(&manifest_path, updated_bytes).map_err(|source| {
            let _ = std::fs::remove_dir_all(&staging_path);
            WorkspaceCommandError::Validation(format!(
                "failed to write updated manifest: {source}"
            ))
        })?;

        warnings.push(format!(
            "project_id remapped from {} to {}",
            original_project_id, new_project_id
        ));
    }

    // Add standard warnings.
    warnings.push(
        "Secret (API Key) is not included in the backup — re-enter credentials after restore"
            .to_string(),
    );

    // Re-validate after remap.
    drop(store);
    let store = OptimizerStore::open(&database_path).map_err(|source| {
        let _ = std::fs::remove_dir_all(&staging_path);
        WorkspaceCommandError::from(source)
    })?;
    store
        .verify_invariants()
        .map_err(|source| {
            let _ = std::fs::remove_dir_all(&staging_path);
            WorkspaceCommandError::from(source)
        })?;

    // Release the SQLite connection before any filesystem move.
    // On Windows, an open file handle prevents `rename` with
    // ERROR_ACCESS_DENIED (os error 5).
    drop(store);

    // If target exists and overwrite=true, remove it.
    if final_path.exists() && request.overwrite {
        std::fs::remove_dir_all(&final_path).map_err(|source| {
            let _ = std::fs::remove_dir_all(&staging_path);
            WorkspaceCommandError::Validation(format!(
                "failed to remove existing target: {source}"
            ))
        })?;
    }

    // Move staging to final. On Windows this may fail if any file handle
    // is still being released; retry a few times with a short delay.
    let mut rename_err = None;
    for attempt in 0..5u32 {
        match std::fs::rename(&staging_path, &final_path) {
            Ok(()) => {
                rename_err = None;
                break;
            }
            Err(source) => {
                rename_err = Some(source);
                if attempt < 4 {
                    std::thread::sleep(std::time::Duration::from_millis(50));
                }
            }
        }
    }
    if let Some(source) = rename_err {
        let _ = std::fs::remove_dir_all(&staging_path);
        return Err(WorkspaceCommandError::Validation(format!(
            "failed to publish restored project: {source}"
        )));
    }

    let updated_manifest = BackupManifest {
        source_project_id: new_project_id.clone(),
        ..manifest
    };

    Ok(ImportProjectBackupResponse {
        schema_version: 1,
        project_id: new_project_id,
        project_path: final_path.to_string_lossy().into_owned(),
        manifest: updated_manifest,
        restored_items,
        warnings,
        source: "backup".to_string(),
    })
}

#[cfg(test)]
mod tests {
    use optimizer_store::{
        OptimizerStore, ProjectSeed, ReviewDecision, ReviewEventKind, ReviewEventRecord,
        ReviewSessionStatus, StoreError,
    };

    use super::{
        StoredProposalHunk, StoredTextAnchor, WorkspaceCommandError, complete_review_decisions,
        is_safe_binding_id, quote_sql_identifier, quote_sql_string_literal, remap_project_id,
        replay_review_decisions,
    };

    fn hunk(id: &str, group: Option<&str>, from: usize, to: usize) -> StoredProposalHunk {
        StoredProposalHunk {
            id: id.into(),
            from: StoredTextAnchor {
                block_id: "block-1".into(),
                offset: from,
            },
            to: StoredTextAnchor {
                block_id: "block-1".into(),
                offset: to,
            },
            original: "a".into(),
            replacement: "b".into(),
            atomic_group: group.map(str::to_owned),
        }
    }

    #[test]
    fn replays_one_persisted_atomic_decision_across_the_whole_group() {
        let hunks = vec![
            hunk("hunk-1", Some("group-1"), 0, 1),
            hunk("hunk-2", Some("group-1"), 1, 2),
            hunk("hunk-3", None, 2, 3),
        ];
        let events = vec![
            ReviewEventRecord {
                id: "event-1".into(),
                proposal_id: "proposal-1".into(),
                sequence: 1,
                base_revision: 0,
                new_revision: 1,
                kind: ReviewEventKind::Decision,
                previous_status: ReviewSessionStatus::Review,
                next_status: ReviewSessionStatus::Review,
                hunk_id: Some("hunk-1".into()),
                decision: Some(ReviewDecision::Accepted),
                payload_json: None,
                occurred_at: "2026-07-16T00:00:00Z".into(),
            },
            ReviewEventRecord {
                id: "event-2".into(),
                proposal_id: "proposal-1".into(),
                sequence: 2,
                base_revision: 1,
                new_revision: 2,
                kind: ReviewEventKind::Decision,
                previous_status: ReviewSessionStatus::Review,
                next_status: ReviewSessionStatus::Ready,
                hunk_id: Some("hunk-3".into()),
                decision: Some(ReviewDecision::Rejected),
                payload_json: None,
                occurred_at: "2026-07-16T00:00:01Z".into(),
            },
        ];
        let decisions =
            complete_review_decisions(&hunks, replay_review_decisions(&hunks, &events).unwrap())
                .unwrap();
        assert_eq!(decisions["hunk-1"], ReviewDecision::Accepted);
        assert_eq!(decisions["hunk-2"], ReviewDecision::Accepted);
        assert_eq!(decisions["hunk-3"], ReviewDecision::Rejected);
    }

    fn seeded_store(project_id: &str) -> OptimizerStore {
        let mut store = OptimizerStore::open_in_memory().unwrap();
        let seed = ProjectSeed {
            project_id: project_id.into(),
            title: "Remap Fixture".into(),
            language: "en".into(),
            initial_commit_id: "commit-1".into(),
            initial_root_hash: "root-1".into(),
            main_branch_id: "branch-main".into(),
            documents: vec![],
            blocks: vec![],
            created_at: "2026-07-16T00:00:00Z".into(),
        };
        store.initialize_project(&seed).unwrap();
        store
    }

    fn immutable_trigger_count(store: &OptimizerStore) -> i64 {
        store
            .connection()
            .query_row(
                "SELECT COUNT(*) FROM sqlite_master \
                 WHERE type = 'trigger' AND name LIKE '%_immutable_%'",
                [],
                |row| row.get(0),
            )
            .unwrap()
    }

    #[test]
    fn remap_project_id_updates_child_rows_and_restores_immutable_triggers() {
        let mut store = seeded_store("project-old");
        let triggers_before = immutable_trigger_count(&store);
        assert!(triggers_before > 0, "fixture schema must ship immutable triggers");

        let updated =
            remap_project_id(store.connection_mut(), "project-old", "project-new").unwrap();
        // project + commit_node + summary_invalidation (project_id and scope_id).
        assert!(updated >= 4, "expected project + children rows remapped, got {updated}");

        let project_id: String = store
            .connection()
            .query_row("SELECT id FROM project", [], |row| row.get(0))
            .unwrap();
        assert_eq!(project_id, "project-new");

        let child_project_id: String = store
            .connection()
            .query_row(
                "SELECT project_id FROM commit_node WHERE id = 'commit-1'",
                [],
                |row| row.get(0),
            )
            .unwrap();
        assert_eq!(child_project_id, "project-new");

        // Special case: summary_invalidation.scope_id must follow the project id.
        let scope_id: String = store
            .connection()
            .query_row(
                "SELECT scope_id FROM summary_invalidation \
                 WHERE scope_type = 'project' LIMIT 1",
                [],
                |row| row.get(0),
            )
            .unwrap();
        assert_eq!(scope_id, "project-new");

        // All triggers must be restored after COMMIT.
        assert_eq!(immutable_trigger_count(&store), triggers_before);

        // Deferred FK checks mean integrity must hold after commit.
        store.verify_invariants().unwrap();
    }

    #[test]
    fn remap_restores_immutable_triggers_that_still_reject_updates() {
        let mut store = seeded_store("project-old");
        remap_project_id(store.connection_mut(), "project-old", "project-new").unwrap();

        // The recreated `commit_node` immutable trigger must be live again.
        let update_result = store.connection().execute(
            "UPDATE commit_node SET root_hash = 'tampered' WHERE id = 'commit-1'",
            [],
        );
        assert!(
            update_result.is_err(),
            "recreated immutable trigger should reject UPDATE"
        );
    }

    #[test]
    fn remap_project_id_rolls_back_and_reports_not_found_for_missing_project() {
        let mut store = OptimizerStore::open_in_memory().unwrap();
        let result = remap_project_id(store.connection_mut(), "missing-old", "project-new");
        match result {
            Err(WorkspaceCommandError::Store(StoreError::NotFound { entity, id })) => {
                assert_eq!(entity, "project");
                assert_eq!(id, "missing-old");
            }
            other => panic!("expected NotFound, got {other:?}"),
        }

        // The failed remap must not have committed any partial state.
        let project_count: i64 = store
            .connection()
            .query_row("SELECT COUNT(*) FROM project", [], |row| row.get(0))
            .unwrap();
        assert_eq!(project_count, 0);
    }

    #[test]
    fn sql_identifier_and_literal_quoting_escapes_hostile_input() {
        // Identifiers are double-quoted with embedded quotes doubled.
        assert_eq!(quote_sql_identifier("a\"b"), "\"a\"\"b\"");
        assert_eq!(quote_sql_identifier("plain_name"), "\"plain_name\"");
        // String literals are single-quoted with embedded quotes doubled.
        assert_eq!(quote_sql_string_literal("o'connor"), "'o''connor'");
        assert_eq!(quote_sql_string_literal("plain"), "'plain'");
    }

    #[test]
    fn binding_id_charset_rejects_whitespace_and_unicode() {
        assert!(is_safe_binding_id("project-abc_123"));
        assert!(is_safe_binding_id("a"));
        assert!(!is_safe_binding_id(""));
        assert!(!is_safe_binding_id("project id"));
        assert!(!is_safe_binding_id("project/id"));
        assert!(!is_safe_binding_id("项目"));
        assert!(!is_safe_binding_id(&"x".repeat(129)));
        assert!(is_safe_binding_id(&"x".repeat(128)));
    }
}
