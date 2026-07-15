use std::fmt;
use std::fmt::Write as _;

use optimizer_store::{
    ApplyBlockEdit, BlockRecord, CommitRecord, DocumentRecord, EditReceipt, OptimizerStore,
    RestoreSnapshot, SnapshotRecord, StoreError,
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
            Self::Validation(_) | Self::NoChanges | Self::Clock => None,
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
    let blocks = store.list_blocks(project_id)?;
    let root_hash = project_root_hash(blocks.iter().map(|block| {
        (
            block.id.as_str(),
            if block.id == current.id {
                content_hash.as_str()
            } else {
                block.content_hash.as_str()
            },
        )
    }));
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
    };
    let receipt = store.apply_block_edit(&command)?;
    save_response(store, project_id, receipt)
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

pub(crate) fn project_root_hash<'a>(
    entries: impl IntoIterator<Item = (&'a str, &'a str)>,
) -> String {
    let mut entries: Vec<_> = entries.into_iter().collect();
    entries.sort_unstable_by(|left, right| left.0.cmp(right.0));
    let mut payload = b"optimizer-project-root-v1\0".to_vec();
    for (block_id, content_hash) in entries {
        payload.extend_from_slice(block_id.len().to_string().as_bytes());
        payload.push(b':');
        payload.extend_from_slice(block_id.as_bytes());
        payload.extend_from_slice(content_hash.len().to_string().as_bytes());
        payload.push(b':');
        payload.extend_from_slice(content_hash.as_bytes());
    }
    sha256(&payload)
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
