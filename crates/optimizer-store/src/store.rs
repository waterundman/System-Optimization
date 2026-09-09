use std::collections::{BTreeMap, BTreeSet};
use std::fs;
use std::path::{Path, PathBuf};
use std::time::{Duration, SystemTime, UNIX_EPOCH};

use rusqlite::{Connection, MAIN_DB, OptionalExtension, Transaction, TransactionBehavior, params};

use crate::error::{StoreError, StoreResult};
use crate::migration::{CURRENT_SCHEMA_VERSION, migrate};
use crate::model::{
    ApplyBlockEdit, ApplyDocumentBatch, BlockRecord, BlockSearchHit, BranchRecord, CommitRecord,
    CreateDocumentReceipt, CreateDocumentWithBlock, CreateKnowledgeItem,
    CreateReviewCandidateBranch, CreateSnapshot, CreateStyleSample, DocumentBatchReceipt,
    DocumentRecord, EditReceipt, KnowledgeItemRecord, ProjectRecord, ProjectSeed, PutSummaryRecord,
    RestoreReceipt, RestoreSnapshot, ReviewCandidateBranchRecord, ReviewEventKind,
    ReviewSessionStatus, SetKnowledgeItemStatus, SetStyleSampleStatus, SnapshotRecord,
    StyleSampleRecord, SummaryInvalidationRecord, SummaryRecord,
};
use crate::operation::append_review_event_in_transaction;
use crate::snapshot::{
    ProjectSnapshotV1, SNAPSHOT_CODEC, SNAPSHOT_CODEC_VERSION, SNAPSHOT_SCHEMA_VERSION,
    SnapshotBlock, SnapshotBranch, SnapshotCommit, SnapshotDocument, SnapshotProject,
    decode_snapshot, encode_snapshot,
};

mod projects;
mod knowledge;
mod documents;
mod commits;
mod maintenance;
pub const MINIMUM_SQLITE_VERSION: &str = "3.51.3";

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct StoreDiagnostics {
    pub sqlite_version: String,
    pub schema_version: i64,
    pub journal_mode: String,
    pub foreign_keys: bool,
    pub synchronous: i64,
    pub migration_backup: Option<PathBuf>,
}

pub struct OptimizerStore {
    pub(crate) connection: Connection,
    migration_backup: Option<PathBuf>,
}

impl OptimizerStore {
    pub fn open(path: impl AsRef<Path>) -> StoreResult<Self> {
        let path = path.as_ref();
        let mut connection = Connection::open(path)?;
        configure(&connection, true)?;
        ensure_safe_sqlite_version(&connection)?;
        let schema_version: i64 =
            connection.query_row("PRAGMA user_version", [], |row| row.get(0))?;
        let migration_backup = if schema_version > 0 && schema_version < CURRENT_SCHEMA_VERSION {
            Some(create_pre_migration_backup(
                &connection,
                path,
                schema_version,
                CURRENT_SCHEMA_VERSION,
            )?)
        } else {
            None
        };
        migrate(&mut connection)?;
        let store = Self {
            connection,
            migration_backup,
        };
        store.verify_invariants()?;
        Ok(store)
    }

    pub fn open_in_memory() -> StoreResult<Self> {
        let mut connection = Connection::open_in_memory()?;
        configure(&connection, false)?;
        ensure_safe_sqlite_version(&connection)?;
        migrate(&mut connection)?;
        Ok(Self {
            connection,
            migration_backup: None,
        })
    }

    pub fn connection(&self) -> &Connection {
        &self.connection
    }

    /// Mutable access to the underlying SQLite connection. Intended for
    /// host-side operations that need the rusqlite transaction API
    /// (e.g. project_id remap during backup import).
    pub fn connection_mut(&mut self) -> &mut Connection {
        &mut self.connection
    }

    pub fn diagnostics(&self) -> StoreResult<StoreDiagnostics> {
        Ok(StoreDiagnostics {
            sqlite_version: self.sqlite_version()?,
            schema_version: self
                .connection
                .query_row("PRAGMA user_version", [], |row| row.get(0))?,
            journal_mode: self
                .connection
                .query_row("PRAGMA journal_mode", [], |row| row.get(0))?,
            foreign_keys: self
                .connection
                .query_row("PRAGMA foreign_keys", [], |row| row.get::<_, i64>(0))?
                == 1,
            synchronous: self
                .connection
                .query_row("PRAGMA synchronous", [], |row| row.get(0))?,
            migration_backup: self.migration_backup.clone(),
        })
    }

    pub fn sqlite_version(&self) -> StoreResult<String> {
        Ok(self
            .connection
            .query_row("SELECT sqlite_version()", [], |row| row.get(0))?)
    }
}

struct EditableBlock {
    project_id: String,
    document_id: String,
    revision: i64,
    content_hash: String,
    content_json: String,
    plain_text: String,
    locked: bool,
}

fn read_block_for_edit(
    transaction: &Transaction<'_>,
    block_id: &str,
) -> StoreResult<EditableBlock> {
    transaction
        .query_row(
            "SELECT d.project_id, b.document_id, b.revision, b.content_hash, b.content_json,
                    b.plain_text, b.locked
             FROM block b JOIN document d ON d.id = b.document_id
             WHERE b.id = ?1 AND b.deleted_at IS NULL",
            [block_id],
            |row| {
                Ok(EditableBlock {
                    project_id: row.get(0)?,
                    document_id: row.get(1)?,
                    revision: row.get(2)?,
                    content_hash: row.get(3)?,
                    content_json: row.get(4)?,
                    plain_text: row.get(5)?,
                    locked: row.get::<_, i64>(6)? == 1,
                })
            },
        )
        .optional()?
        .ok_or_else(|| StoreError::NotFound {
            entity: "block",
            id: block_id.to_owned(),
        })
}

fn create_pre_migration_backup(
    connection: &Connection,
    database_path: &Path,
    from_version: i64,
    to_version: i64,
) -> StoreResult<PathBuf> {
    let file_name = database_path
        .file_name()
        .ok_or_else(|| StoreError::Validation("database path has no file name".into()))?
        .to_string_lossy();
    let nonce = SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .map_err(|error| StoreError::Validation(format!("system clock is invalid: {error}")))?
        .as_nanos();
    let destination = database_path
        .parent()
        .unwrap_or_else(|| Path::new("."))
        .join(format!(
            "{file_name}.pre-v{from_version}-to-v{to_version}-{nonce}.sqlite3"
        ));
    connection.backup(MAIN_DB, &destination, None)?;
    let backup = Connection::open(&destination)?;
    let result: String = backup.query_row("PRAGMA quick_check", [], |row| row.get(0))?;
    drop(backup);
    if result != "ok" {
        let _ = fs::remove_file(&destination);
        return Err(StoreError::InvariantViolation(format!(
            "pre-migration backup quick_check returned {result}"
        )));
    }
    Ok(destination)
}

fn configure(connection: &Connection, wal: bool) -> StoreResult<()> {
    connection.busy_timeout(Duration::from_secs(5))?;
    connection.pragma_update(None, "foreign_keys", "ON")?;
    connection.pragma_update(None, "synchronous", "FULL")?;
    connection.pragma_update(None, "wal_autocheckpoint", 1000)?;
    if wal {
        let mode: String = connection.query_row("PRAGMA journal_mode=WAL", [], |row| row.get(0))?;
        if !mode.eq_ignore_ascii_case("wal") {
            return Err(StoreError::Validation(format!(
                "failed to enable WAL; SQLite returned {mode}"
            )));
        }
    }
    Ok(())
}

fn ensure_safe_sqlite_version(connection: &Connection) -> StoreResult<()> {
    let actual: String = connection.query_row("SELECT sqlite_version()", [], |row| row.get(0))?;
    let parsed = parse_version(&actual).ok_or_else(|| StoreError::UnsafeSqliteVersion {
        actual: actual.clone(),
        minimum: MINIMUM_SQLITE_VERSION,
    })?;
    if parsed < (3, 51, 3) {
        return Err(StoreError::UnsafeSqliteVersion {
            actual,
            minimum: MINIMUM_SQLITE_VERSION,
        });
    }
    Ok(())
}

fn parse_version(value: &str) -> Option<(u64, u64, u64)> {
    let mut values = value.split('.').take(3).map(str::parse::<u64>);
    Some((
        values.next()?.ok()?,
        values.next()?.ok()?,
        values.next()?.ok()?,
    ))
}

fn validate_seed(seed: &ProjectSeed) -> StoreResult<()> {
    for (field, value) in [
        ("project_id", seed.project_id.as_str()),
        ("title", seed.title.as_str()),
        ("language", seed.language.as_str()),
        ("initial_commit_id", seed.initial_commit_id.as_str()),
        ("initial_root_hash", seed.initial_root_hash.as_str()),
        ("main_branch_id", seed.main_branch_id.as_str()),
        ("created_at", seed.created_at.as_str()),
    ] {
        if value.trim().is_empty() {
            return Err(StoreError::Validation(format!("{field} must not be empty")));
        }
    }
    Ok(())
}

fn validate_edit(command: &ApplyBlockEdit) -> StoreResult<()> {
    if command.expected_revision < 0 {
        return Err(StoreError::Validation(
            "expected_revision must be non-negative".into(),
        ));
    }
    for (field, value) in [
        ("edit_id", command.edit_id.as_str()),
        ("commit_id", command.commit_id.as_str()),
        ("branch_id", command.branch_id.as_str()),
        (
            "expected_head_commit_id",
            command.expected_head_commit_id.as_str(),
        ),
        ("block_id", command.block_id.as_str()),
        ("expected_hash", command.expected_hash.as_str()),
        ("new_content_hash", command.new_content_hash.as_str()),
        ("new_root_hash", command.new_root_hash.as_str()),
        ("reason", command.reason.as_str()),
        ("actor_type", command.actor_type.as_str()),
        ("occurred_at", command.occurred_at.as_str()),
    ] {
        if value.trim().is_empty() {
            return Err(StoreError::Validation(format!("{field} must not be empty")));
        }
    }
    if command.expected_project_revision < 0 {
        return Err(StoreError::Validation(
            "expected_project_revision must be non-negative".into(),
        ));
    }
    Ok(())
}

fn validate_review_candidate_branch(command: &CreateReviewCandidateBranch) -> StoreResult<()> {
    if command.expected_review_revision < 0 || command.expected_block_revision < 0 {
        return Err(StoreError::Validation(
            "candidate review and block revisions must be non-negative".into(),
        ));
    }
    for (field, value) in [
        ("proposal_id", command.proposal_id.as_str()),
        ("project_id", command.project_id.as_str()),
        (
            "expected_project_head_commit_id",
            command.expected_project_head_commit_id.as_str(),
        ),
        ("branch_id", command.branch_id.as_str()),
        ("branch_name", command.branch_name.as_str()),
        ("commit_id", command.commit_id.as_str()),
        ("snapshot_id", command.snapshot_id.as_str()),
        ("target_document_id", command.target_document_id.as_str()),
        ("target_block_id", command.target_block_id.as_str()),
        ("expected_block_hash", command.expected_block_hash.as_str()),
        ("new_content_hash", command.new_content_hash.as_str()),
        ("new_root_hash", command.new_root_hash.as_str()),
        ("occurred_at", command.occurred_at.as_str()),
    ] {
        if value.trim().is_empty() {
            return Err(StoreError::Validation(format!("{field} must not be empty")));
        }
    }
    if command.branch_name.trim() != command.branch_name
        || command.branch_name.chars().count() > 120
    {
        return Err(StoreError::Validation(
            "candidate branch name must be trimmed and at most 120 characters".into(),
        ));
    }
    if command.expected_block_hash == command.new_content_hash
        || command.new_plain_text.len() > 8 * 1024 * 1024
        || command.new_content_json.len() > 16 * 1024 * 1024
    {
        return Err(StoreError::Validation(
            "candidate branch content is unchanged or exceeds the storage bound".into(),
        ));
    }
    let content: serde_json::Value =
        serde_json::from_str(&command.new_content_json).map_err(|error| {
            StoreError::Validation(format!("candidate content JSON is invalid: {error}"))
        })?;
    if !content.is_object() {
        return Err(StoreError::Validation(
            "candidate content JSON must be an object".into(),
        ));
    }
    Ok(())
}

fn validate_create_style_sample(command: &CreateStyleSample) -> StoreResult<()> {
    for (field, value) in [
        ("id", command.id.as_str()),
        ("project_id", command.project_id.as_str()),
        ("title", command.title.as_str()),
        ("content", command.content.as_str()),
        ("content_hash", command.content_hash.as_str()),
        ("sensitivity", command.sensitivity.as_str()),
        ("created_at", command.created_at.as_str()),
    ] {
        if value.trim().is_empty() {
            return Err(StoreError::Validation(format!("{field} must not be empty")));
        }
    }
    if !matches!(
        command.sensitivity.as_str(),
        "local_sensitive" | "never_send"
    ) {
        return Err(StoreError::Validation(
            "style sample sensitivity is unsupported".into(),
        ));
    }
    Ok(())
}

fn validate_create_knowledge_item(command: &CreateKnowledgeItem) -> StoreResult<()> {
    for (field, value) in [
        ("id", command.id.as_str()),
        ("project_id", command.project_id.as_str()),
        ("kind", command.kind.as_str()),
        ("title", command.title.as_str()),
        ("content", command.content.as_str()),
        ("content_hash", command.content_hash.as_str()),
        ("authority", command.authority.as_str()),
        ("sensitivity", command.sensitivity.as_str()),
        ("created_at", command.created_at.as_str()),
    ] {
        if value.trim().is_empty() {
            return Err(StoreError::Validation(format!("{field} must not be empty")));
        }
    }
    if !matches!(command.kind.as_str(), "fact" | "constraint") {
        return Err(StoreError::Validation(
            "knowledge item kind is unsupported".into(),
        ));
    }
    if !matches!(
        command.authority.as_str(),
        "user_confirmed" | "source_derived" | "model_inferred" | "external_untrusted"
    ) {
        return Err(StoreError::Validation(
            "knowledge item authority is unsupported".into(),
        ));
    }
    if !matches!(
        command.sensitivity.as_str(),
        "public" | "local" | "local_sensitive" | "never_send"
    ) {
        return Err(StoreError::Validation(
            "knowledge item sensitivity is unsupported".into(),
        ));
    }
    let severity_is_valid = match command.kind.as_str() {
        "fact" => command.severity.is_none(),
        "constraint" => matches!(command.severity.as_deref(), Some("hard" | "soft")),
        _ => false,
    };
    if !severity_is_valid {
        return Err(StoreError::Validation(
            "knowledge item severity does not match its kind".into(),
        ));
    }
    Ok(())
}

fn validate_create_document(command: &CreateDocumentWithBlock) -> StoreResult<()> {
    for (field, value) in [
        ("document_id", command.document_id.as_str()),
        ("document_kind", command.document_kind.as_str()),
        ("document_title", command.document_title.as_str()),
        ("document_order_key", command.document_order_key.as_str()),
        ("block_id", command.block_id.as_str()),
        ("block_kind", command.block_kind.as_str()),
        ("block_order_key", command.block_order_key.as_str()),
        ("block_content_json", command.block_content_json.as_str()),
        ("block_content_hash", command.block_content_hash.as_str()),
        ("commit_id", command.commit_id.as_str()),
        ("branch_id", command.branch_id.as_str()),
        (
            "expected_head_commit_id",
            command.expected_head_commit_id.as_str(),
        ),
        ("new_root_hash", command.new_root_hash.as_str()),
        ("actor_type", command.actor_type.as_str()),
        ("occurred_at", command.occurred_at.as_str()),
    ] {
        if value.trim().is_empty() {
            return Err(StoreError::Validation(format!("{field} must not be empty")));
        }
    }
    if command.expected_project_revision < 0 {
        return Err(StoreError::Validation(
            "expected_project_revision must be non-negative".into(),
        ));
    }
    Ok(())
}

fn validate_document_batch(command: &ApplyDocumentBatch) -> StoreResult<()> {
    if command.mutations.is_empty() || command.mutations.len() > 1_000 {
        return Err(StoreError::Validation(
            "document mutations must contain 1 to 1000 items".into(),
        ));
    }
    if command.expected_project_revision < 0 {
        return Err(StoreError::Validation(
            "expected_project_revision must be non-negative".into(),
        ));
    }
    for (field, value) in [
        ("commit_id", command.commit_id.as_str()),
        ("branch_id", command.branch_id.as_str()),
        (
            "expected_head_commit_id",
            command.expected_head_commit_id.as_str(),
        ),
        ("new_root_hash", command.new_root_hash.as_str()),
        ("reason", command.reason.as_str()),
        ("actor_type", command.actor_type.as_str()),
        ("occurred_at", command.occurred_at.as_str()),
    ] {
        if value.trim().is_empty() {
            return Err(StoreError::Validation(format!("{field} must not be empty")));
        }
    }
    if !matches!(
        command.reason.as_str(),
        "document_rename"
            | "document_reorder"
            | "document_reparent"
            | "document_archive"
            | "document_restore"
    ) {
        return Err(StoreError::Validation(
            "document mutation reason is unsupported".into(),
        ));
    }
    let mut ids = BTreeSet::new();
    for mutation in &command.mutations {
        if mutation.expected_revision < 0 {
            return Err(StoreError::Validation(
                "document expected_revision must be non-negative".into(),
            ));
        }
        for (field, value) in [
            ("document_id", mutation.document_id.as_str()),
            ("kind", mutation.kind.as_str()),
            ("title", mutation.title.as_str()),
            ("order_key", mutation.order_key.as_str()),
            ("before_hash", mutation.before_hash.as_str()),
            ("after_hash", mutation.after_hash.as_str()),
            ("operation", mutation.operation.as_str()),
        ] {
            if value.trim().is_empty() {
                return Err(StoreError::Validation(format!(
                    "document mutation {field} must not be empty"
                )));
            }
        }
        if !ids.insert(mutation.document_id.as_str()) {
            return Err(StoreError::Validation(
                "document mutation IDs must be unique".into(),
            ));
        }
        if !matches!(
            mutation.operation.as_str(),
            "rename" | "reorder" | "reparent" | "archive" | "restore"
        ) {
            return Err(StoreError::Validation(
                "document mutation operation is unsupported".into(),
            ));
        }
        if mutation.before_hash == mutation.after_hash {
            return Err(StoreError::Validation(
                "document mutation must change its state hash".into(),
            ));
        }
    }
    Ok(())
}

fn validate_final_document_tree(documents: &[DocumentRecord]) -> StoreResult<()> {
    if !documents
        .iter()
        .any(|document| document.deleted_at.is_none())
    {
        return Err(StoreError::Validation(
            "a project must retain at least one active document".into(),
        ));
    }
    let by_id = documents
        .iter()
        .map(|document| (document.id.as_str(), document))
        .collect::<BTreeMap<_, _>>();
    let mut order_keys = BTreeSet::new();
    for document in documents {
        if !order_keys.insert((document.parent_id.as_deref(), document.order_key.as_str())) {
            return Err(StoreError::Validation(
                "document order keys must be unique within each parent".into(),
            ));
        }
        if document.deleted_at.is_some() {
            continue;
        }
        let mut cursor = document.parent_id.as_deref();
        let mut visited = BTreeSet::from([document.id.as_str()]);
        while let Some(parent_id) = cursor {
            let parent = by_id.get(parent_id).ok_or_else(|| {
                StoreError::Validation("document parent does not exist in the project".into())
            })?;
            if parent.deleted_at.is_some() {
                return Err(StoreError::Validation(
                    "an active document cannot have an archived parent".into(),
                ));
            }
            if !visited.insert(parent.id.as_str()) {
                return Err(StoreError::Validation(
                    "document hierarchy contains a cycle".into(),
                ));
            }
            cursor = parent.parent_id.as_deref();
        }
    }
    Ok(())
}

fn validate_put_summary_record(command: &PutSummaryRecord) -> StoreResult<()> {
    for (field, value) in [
        ("project_id", command.project_id.as_str()),
        ("scope_type", command.scope_type.as_str()),
        ("scope_id", command.scope_id.as_str()),
        (
            "expected_source_commit_id",
            command.expected_source_commit_id.as_str(),
        ),
        ("source_hash", command.source_hash.as_str()),
        ("summary", command.summary.as_str()),
        ("summary_hash", command.summary_hash.as_str()),
        ("provider_id", command.provider_id.as_str()),
        ("model", command.model.as_str()),
        ("generated_at", command.generated_at.as_str()),
    ] {
        if value.trim().is_empty() {
            return Err(StoreError::Validation(format!(
                "summary {field} must not be empty"
            )));
        }
    }
    if !matches!(
        command.scope_type.as_str(),
        "block" | "document" | "project"
    ) {
        return Err(StoreError::Validation(
            "summary scope type is unsupported".into(),
        ));
    }
    if command.summary.len() > 2 * 1024 * 1024 {
        return Err(StoreError::Validation("summary text exceeds 2 MiB".into()));
    }
    Ok(())
}

fn validate_summary_scope_reference(
    project_id: &str,
    scope_type: &str,
    scope_id: &str,
) -> StoreResult<()> {
    if project_id.trim().is_empty() || scope_id.trim().is_empty() {
        return Err(StoreError::Validation(
            "project id and summary scope id are required".into(),
        ));
    }
    if !matches!(scope_type, "block" | "document" | "project") {
        return Err(StoreError::Validation(
            "summary scope type is unsupported".into(),
        ));
    }
    if scope_type == "project" && project_id != scope_id {
        return Err(StoreError::Validation(
            "project summary scope must use the project id".into(),
        ));
    }
    Ok(())
}

fn validate_active_summary_scope(
    transaction: &Transaction<'_>,
    project_id: &str,
    scope_type: &str,
    scope_id: &str,
) -> StoreResult<()> {
    let exists: bool = match scope_type {
        "project" => {
            if project_id != scope_id {
                return Err(StoreError::Validation(
                    "project summary scope must use the project id".into(),
                ));
            }
            transaction.query_row(
                "SELECT EXISTS(SELECT 1 FROM project WHERE id = ?1)",
                [project_id],
                |row| row.get(0),
            )?
        }
        "document" => transaction.query_row(
            "SELECT EXISTS(
               SELECT 1 FROM document
               WHERE project_id = ?1 AND id = ?2 AND deleted_at IS NULL
             )",
            params![project_id, scope_id],
            |row| row.get(0),
        )?,
        "block" => transaction.query_row(
            "SELECT EXISTS(
               SELECT 1 FROM block AS b
               JOIN document AS d ON d.id = b.document_id
               WHERE d.project_id = ?1 AND b.id = ?2
                 AND b.deleted_at IS NULL AND d.deleted_at IS NULL
             )",
            params![project_id, scope_id],
            |row| row.get(0),
        )?,
        _ => {
            return Err(StoreError::Validation(
                "summary scope type is unsupported".into(),
            ));
        }
    };
    if !exists {
        return Err(StoreError::NotFound {
            entity: "summary_scope",
            id: format!("{scope_type}:{scope_id}"),
        });
    }
    Ok(())
}

fn enqueue_summary_invalidation(
    transaction: &Transaction<'_>,
    project_id: &str,
    scope_type: &str,
    scope_id: &str,
    source_commit_id: &str,
    reason: &str,
    created_at: &str,
) -> StoreResult<()> {
    transaction.execute(
        "INSERT INTO summary_invalidation(
           project_id, scope_type, scope_id, source_commit_id, reason,
           invalidation_count, created_at
         ) VALUES (?1, ?2, ?3, ?4, ?5, 1, ?6)
         ON CONFLICT(project_id, scope_type, scope_id) DO UPDATE SET
           source_commit_id = excluded.source_commit_id,
           reason = excluded.reason,
           invalidation_count = summary_invalidation.invalidation_count + 1,
           created_at = excluded.created_at",
        params![
            project_id,
            scope_type,
            scope_id,
            source_commit_id,
            reason,
            created_at,
        ],
    )?;
    Ok(())
}

fn read_summary_record(
    transaction: &Transaction<'_>,
    project_id: &str,
    scope_type: &str,
    scope_id: &str,
) -> StoreResult<SummaryRecord> {
    transaction
        .query_row(
            "SELECT project_id, scope_type, scope_id, source_commit_id, source_hash,
                    summary_text, summary_hash, provider_id, model, revision,
                    generated_at, updated_at
             FROM summary_record
             WHERE project_id = ?1 AND scope_type = ?2 AND scope_id = ?3",
            params![project_id, scope_type, scope_id],
            map_summary_record,
        )
        .optional()?
        .ok_or_else(|| StoreError::NotFound {
            entity: "summary_record",
            id: format!("{scope_type}:{scope_id}"),
        })
}

fn map_summary_record(row: &rusqlite::Row<'_>) -> rusqlite::Result<SummaryRecord> {
    Ok(SummaryRecord {
        project_id: row.get(0)?,
        scope_type: row.get(1)?,
        scope_id: row.get(2)?,
        source_commit_id: row.get(3)?,
        source_hash: row.get(4)?,
        summary: row.get(5)?,
        summary_hash: row.get(6)?,
        provider_id: row.get(7)?,
        model: row.get(8)?,
        revision: row.get(9)?,
        generated_at: row.get(10)?,
        updated_at: row.get(11)?,
    })
}

fn validate_set_style_sample_status(command: &SetStyleSampleStatus) -> StoreResult<()> {
    if command.expected_revision < 0 {
        return Err(StoreError::Validation(
            "expected_revision must be non-negative".into(),
        ));
    }
    for (field, value) in [
        ("project_id", command.project_id.as_str()),
        ("id", command.id.as_str()),
        ("status", command.status.as_str()),
        ("updated_at", command.updated_at.as_str()),
    ] {
        if value.trim().is_empty() {
            return Err(StoreError::Validation(format!("{field} must not be empty")));
        }
    }
    if !matches!(command.status.as_str(), "canonical" | "archived") {
        return Err(StoreError::Validation(
            "style sample status is unsupported".into(),
        ));
    }
    Ok(())
}

fn validate_set_knowledge_item_status(command: &SetKnowledgeItemStatus) -> StoreResult<()> {
    if command.expected_revision < 0 {
        return Err(StoreError::Validation(
            "expected_revision must be non-negative".into(),
        ));
    }
    for (field, value) in [
        ("project_id", command.project_id.as_str()),
        ("id", command.id.as_str()),
        ("status", command.status.as_str()),
        ("updated_at", command.updated_at.as_str()),
    ] {
        if value.trim().is_empty() {
            return Err(StoreError::Validation(format!("{field} must not be empty")));
        }
    }
    if !matches!(
        command.status.as_str(),
        "canonical" | "archived" | "rejected"
    ) {
        return Err(StoreError::Validation(
            "knowledge item status is unsupported".into(),
        ));
    }
    Ok(())
}

fn map_document_record(row: &rusqlite::Row<'_>) -> rusqlite::Result<DocumentRecord> {
    Ok(DocumentRecord {
        id: row.get(0)?,
        project_id: row.get(1)?,
        parent_id: row.get(2)?,
        kind: row.get(3)?,
        title: row.get(4)?,
        order_key: row.get(5)?,
        revision: row.get(6)?,
        deleted_at: row.get(7)?,
    })
}

fn map_style_sample(row: &rusqlite::Row<'_>) -> rusqlite::Result<StyleSampleRecord> {
    Ok(StyleSampleRecord {
        id: row.get(0)?,
        project_id: row.get(1)?,
        title: row.get(2)?,
        content: row.get(3)?,
        content_hash: row.get(4)?,
        status: row.get(5)?,
        sensitivity: row.get(6)?,
        revision: row.get(7)?,
        created_at: row.get(8)?,
        updated_at: row.get(9)?,
    })
}

fn map_knowledge_item(row: &rusqlite::Row<'_>) -> rusqlite::Result<KnowledgeItemRecord> {
    Ok(KnowledgeItemRecord {
        id: row.get(0)?,
        project_id: row.get(1)?,
        kind: row.get(2)?,
        title: row.get(3)?,
        content: row.get(4)?,
        content_hash: row.get(5)?,
        status: row.get(6)?,
        authority: row.get(7)?,
        sensitivity: row.get(8)?,
        severity: row.get(9)?,
        revision: row.get(10)?,
        created_at: row.get(11)?,
        updated_at: row.get(12)?,
    })
}

fn read_style_sample(
    transaction: &Transaction<'_>,
    project_id: &str,
    id: &str,
) -> StoreResult<StyleSampleRecord> {
    transaction
        .query_row(
            "SELECT id, project_id, title, content, content_hash, status, sensitivity,
                    revision, created_at, updated_at
             FROM style_sample WHERE project_id = ?1 AND id = ?2",
            params![project_id, id],
            map_style_sample,
        )
        .optional()?
        .ok_or_else(|| StoreError::NotFound {
            entity: "style_sample",
            id: id.to_owned(),
        })
}

fn read_knowledge_item(
    transaction: &Transaction<'_>,
    project_id: &str,
    id: &str,
) -> StoreResult<KnowledgeItemRecord> {
    transaction
        .query_row(
            "SELECT id, project_id, kind, title, content, content_hash, status,
                    authority, sensitivity, severity, revision, created_at, updated_at
             FROM knowledge_item WHERE project_id = ?1 AND id = ?2",
            params![project_id, id],
            map_knowledge_item,
        )
        .optional()?
        .ok_or_else(|| StoreError::NotFound {
            entity: "knowledge_item",
            id: id.to_owned(),
        })
}

fn map_block_record(row: &rusqlite::Row<'_>) -> rusqlite::Result<BlockRecord> {
    Ok(BlockRecord {
        id: row.get(0)?,
        document_id: row.get(1)?,
        kind: row.get(2)?,
        order_key: row.get(3)?,
        content_json: row.get(4)?,
        plain_text: row.get(5)?,
        content_hash: row.get(6)?,
        revision: row.get(7)?,
        locked: row.get::<_, i64>(8)? == 1,
    })
}

fn validate_restore(command: &RestoreSnapshot) -> StoreResult<()> {
    for (field, value) in [
        ("snapshot_id", command.snapshot_id.as_str()),
        ("branch_id", command.branch_id.as_str()),
        ("new_commit_id", command.new_commit_id.as_str()),
        ("edit_id_prefix", command.edit_id_prefix.as_str()),
        ("actor_type", command.actor_type.as_str()),
        ("occurred_at", command.occurred_at.as_str()),
    ] {
        if value.trim().is_empty() {
            return Err(StoreError::Validation(format!("{field} must not be empty")));
        }
    }
    Ok(())
}

fn read_head_snapshot(
    transaction: &Transaction<'_>,
    project_id: &str,
) -> StoreResult<ProjectSnapshotV1> {
    let (project_title, project_language, head_commit_id): (String, String, String) = transaction
        .query_row(
            "SELECT title, language, head_commit_id FROM project WHERE id = ?1",
            [project_id],
            |row| Ok((row.get(0)?, row.get(1)?, row.get(2)?)),
        )
        .optional()?
        .ok_or_else(|| StoreError::NotFound {
            entity: "project",
            id: project_id.to_owned(),
        })?;
    let root_hash: String = transaction.query_row(
        "SELECT root_hash FROM commit_node WHERE id = ?1 AND project_id = ?2",
        params![head_commit_id, project_id],
        |row| row.get(0),
    )?;

    let branches = {
        let mut statement = transaction.prepare(
            "SELECT id, name, head_commit_id FROM branch WHERE project_id = ?1 ORDER BY id",
        )?;
        statement
            .query_map([project_id], |row| {
                Ok(SnapshotBranch {
                    id: row.get(0)?,
                    name: row.get(1)?,
                    head_commit_id: row.get(2)?,
                })
            })?
            .collect::<Result<_, _>>()?
    };
    let documents = read_active_documents(transaction, project_id)?;
    let blocks = read_active_snapshot_blocks(transaction, project_id)?;

    Ok(ProjectSnapshotV1 {
        schema_version: SNAPSHOT_SCHEMA_VERSION,
        project: SnapshotProject {
            id: project_id.to_owned(),
            title: project_title,
            language: project_language,
        },
        commit: SnapshotCommit {
            id: head_commit_id,
            root_hash,
        },
        branches,
        documents,
        blocks,
    })
}

fn read_active_documents(
    transaction: &Transaction<'_>,
    project_id: &str,
) -> StoreResult<Vec<SnapshotDocument>> {
    let mut statement = transaction.prepare(
        "SELECT id, parent_id, kind, title, order_key, revision
         FROM document WHERE project_id = ?1 AND deleted_at IS NULL ORDER BY id",
    )?;
    Ok(statement
        .query_map([project_id], |row| {
            Ok(SnapshotDocument {
                id: row.get(0)?,
                parent_id: row.get(1)?,
                kind: row.get(2)?,
                title: row.get(3)?,
                order_key: row.get(4)?,
                revision: row.get(5)?,
            })
        })?
        .collect::<Result<_, _>>()?)
}

fn read_active_snapshot_blocks(
    transaction: &Transaction<'_>,
    project_id: &str,
) -> StoreResult<Vec<SnapshotBlock>> {
    let mut statement = transaction.prepare(
        "SELECT b.id, b.document_id, b.kind, b.order_key, b.content_json, b.plain_text,
                b.content_hash, b.revision, b.locked
         FROM block b JOIN document d ON d.id = b.document_id
         WHERE d.project_id = ?1 AND b.deleted_at IS NULL AND d.deleted_at IS NULL
         ORDER BY b.id",
    )?;
    Ok(statement
        .query_map([project_id], |row| {
            Ok(SnapshotBlock {
                id: row.get(0)?,
                document_id: row.get(1)?,
                kind: row.get(2)?,
                order_key: row.get(3)?,
                content_json: row.get(4)?,
                plain_text: row.get(5)?,
                content_hash: row.get(6)?,
                revision: row.get(7)?,
                locked: row.get::<_, i64>(8)? == 1,
            })
        })?
        .collect::<Result<_, _>>()?)
}


#[cfg(test)]
mod migration_backup_tests {
    use std::fs;
    use std::path::PathBuf;
    use std::time::{SystemTime, UNIX_EPOCH};

    use rusqlite::Connection;

    use super::OptimizerStore;
    use crate::migration::{CURRENT_SCHEMA_VERSION, MIGRATION_1};

    struct TempDirectory(PathBuf);

    impl TempDirectory {
        fn new() -> Self {
            let nonce = SystemTime::now()
                .duration_since(UNIX_EPOCH)
                .unwrap()
                .as_nanos();
            let path = std::env::temp_dir().join(format!(
                "optimizer-migration-backup-{}-{nonce}",
                std::process::id()
            ));
            fs::create_dir_all(&path).unwrap();
            Self(path)
        }
    }

    impl Drop for TempDirectory {
        fn drop(&mut self) {
            let _ = fs::remove_dir_all(&self.0);
        }
    }

    #[test]
    fn creates_a_checked_version_one_backup_before_file_migration() {
        let temp = TempDirectory::new();
        let database = temp.0.join("project.sqlite3");
        let connection = Connection::open(&database).unwrap();
        connection.execute_batch(MIGRATION_1).unwrap();
        connection
            .execute(
                "INSERT INTO schema_migration(version, applied_at) VALUES (1, '2026-07-14T00:00:00.000Z')",
                [],
            )
            .unwrap();
        connection.pragma_update(None, "user_version", 1).unwrap();
        drop(connection);

        let store = OptimizerStore::open(&database).unwrap();
        let diagnostics = store.diagnostics().unwrap();
        assert_eq!(diagnostics.schema_version, CURRENT_SCHEMA_VERSION);
        let backup_path = diagnostics.migration_backup.unwrap();
        assert!(backup_path.exists());
        let backup = Connection::open(backup_path).unwrap();
        let backup_version: i64 = backup
            .query_row("PRAGMA user_version", [], |row| row.get(0))
            .unwrap();
        let operation_tables: i64 = backup
            .query_row(
                "SELECT COUNT(*) FROM sqlite_schema WHERE type = 'table' AND name = 'operation_run'",
                [],
                |row| row.get(0),
            )
            .unwrap();
        assert_eq!(backup_version, 1);
        assert_eq!(operation_tables, 0);
    }
}
