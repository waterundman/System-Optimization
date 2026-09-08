use std::collections::{BTreeMap, BTreeSet};
use std::fmt;
use std::fmt::Write as _;

use optimizer_store::{
    ApplyDocumentBatch, BlockRecord, CreateDocumentWithBlock, DocumentMutation, DocumentRecord,
    EditReceipt, OptimizerStore, StoreError,
};
use serde::Serialize;
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

mod workspace_commands_backup;
mod workspace_commands_items;
mod workspace_commands_review;

pub use workspace_commands_backup::{
    ExportProjectBackupRequest, ExportProjectBackupResponse, ImportProjectBackupRequest,
    ImportProjectBackupResponse,
};
pub(crate) use workspace_commands_backup::{export_project_backup, import_project_backup};
pub(crate) use workspace_commands_items::{
    create_knowledge_item, create_style_sample, knowledge_context, list_knowledge_items,
    list_style_samples, save_block, set_knowledge_item_status, set_style_sample_status,
};
pub use workspace_commands_review::CompareDocumentsSpec;
pub(crate) use workspace_commands_review::{
    apply_reviewed_proposal, compare_documents, create_checkpoint, create_review_candidate_branch,
    list_review_candidates, load_review_candidate, load_version_history, restore_checkpoint,
};
pub use workspace_commands_review::list_timeline_events;
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
#[cfg(test)]
mod tests {
    use optimizer_store::{
        OptimizerStore, ProjectSeed, ReviewDecision, ReviewEventKind, ReviewEventRecord,
        ReviewSessionStatus, StoreError,
    };

    use super::WorkspaceCommandError;
    use super::workspace_commands_backup::{
        is_safe_binding_id, quote_sql_identifier, quote_sql_string_literal, remap_project_id,
    };
    use super::workspace_commands_review::{
        StoredProposalHunk, StoredTextAnchor, complete_review_decisions, replay_review_decisions,
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
