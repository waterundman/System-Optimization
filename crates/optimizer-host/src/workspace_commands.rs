use std::collections::{BTreeMap, BTreeSet};
use std::fmt;
use std::fmt::Write as _;

use optimizer_store::{
    AppendReviewEvent, ApplyBlockEdit, ApplyDocumentBatch, BlockRecord, CommitRecord,
    CreateDocumentWithBlock, CreateStyleSample, DocumentMutation, DocumentRecord, EditReceipt,
    OperationArtifactKind, OptimizerStore, RestoreSnapshot, ReviewDecision, ReviewEventKind,
    ReviewSessionStatus, SetStyleSampleStatus, SnapshotRecord, StoreError, StyleSampleRecord,
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
        .expect("active document is present in its sibling group");
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
        .expect("active document is present in its sibling group");

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
                .expect("active parent is present in its sibling group");
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
    let mut decisions = BTreeMap::new();
    for event in events {
        if event.kind == ReviewEventKind::Decision {
            let hunk_id = event.hunk_id.ok_or_else(|| {
                WorkspaceCommandError::Validation("Decision event has no hunk id".into())
            })?;
            let decision = event.decision.ok_or_else(|| {
                WorkspaceCommandError::Validation("Decision event has no decision".into())
            })?;
            decisions.insert(hunk_id, decision);
        }
    }
    let hunk_ids = proposal
        .hunks
        .iter()
        .map(|hunk| hunk.id.as_str())
        .collect::<BTreeSet<_>>();
    if decisions.len() != proposal.hunks.len()
        || decisions.keys().any(|id| !hunk_ids.contains(id.as_str()))
    {
        return Err(WorkspaceCommandError::Validation(
            "Review decisions do not cover exactly every proposal hunk".into(),
        ));
    }
    validate_atomic_decisions(&proposal.hunks, &decisions)?;

    let mut accepted = Vec::new();
    let mut rejected = Vec::new();
    for hunk in &proposal.hunks {
        match decisions.get(&hunk.id) {
            Some(ReviewDecision::Accepted) => accepted.push(hunk),
            Some(ReviewDecision::Rejected) => rejected.push(hunk.id.clone()),
            None => unreachable!("coverage checked above"),
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
    if !matches!(record.status.as_str(), "canonical" | "archived")
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
