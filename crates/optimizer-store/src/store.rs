use std::collections::{BTreeMap, BTreeSet};
use std::fs;
use std::path::{Path, PathBuf};
use std::time::{Duration, SystemTime, UNIX_EPOCH};

use rusqlite::{Connection, MAIN_DB, OptionalExtension, Transaction, TransactionBehavior, params};

use crate::error::{StoreError, StoreResult};
use crate::migration::{CURRENT_SCHEMA_VERSION, migrate};
use crate::model::{
    ApplyBlockEdit, ApplyDocumentBatch, BlockRecord, BlockSearchHit, BranchRecord, CommitRecord,
    CreateDocumentReceipt, CreateDocumentWithBlock, CreateSnapshot, CreateStyleSample,
    DocumentBatchReceipt, DocumentRecord, EditReceipt, ProjectRecord, ProjectSeed,
    PutSummaryRecord, RestoreReceipt, RestoreSnapshot, ReviewEventKind, ReviewSessionStatus,
    SetStyleSampleStatus, SnapshotRecord, StyleSampleRecord, SummaryInvalidationRecord,
    SummaryRecord,
};
use crate::operation::append_review_event_in_transaction;
use crate::snapshot::{
    ProjectSnapshotV1, SNAPSHOT_CODEC, SNAPSHOT_CODEC_VERSION, SNAPSHOT_SCHEMA_VERSION,
    SnapshotBlock, SnapshotBranch, SnapshotCommit, SnapshotDocument, SnapshotProject,
    decode_snapshot, encode_snapshot,
};

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

    pub fn initialize_project(&mut self, seed: &ProjectSeed) -> StoreResult<()> {
        validate_seed(seed)?;
        let transaction = self
            .connection
            .transaction_with_behavior(TransactionBehavior::Immediate)?;
        transaction.execute(
            "INSERT INTO project(id, title, language, schema_version, head_commit_id, revision, created_at, updated_at)
             VALUES (?1, ?2, ?3, ?4, NULL, 0, ?5, ?5)",
            params![seed.project_id, seed.title, seed.language, CURRENT_SCHEMA_VERSION, seed.created_at],
        )?;

        for document in &seed.documents {
            transaction.execute(
                "INSERT INTO document(id, project_id, parent_id, kind, title, order_key, revision)
                 VALUES (?1, ?2, ?3, ?4, ?5, ?6, 0)",
                params![
                    document.id,
                    seed.project_id,
                    document.parent_id,
                    document.kind,
                    document.title,
                    document.order_key
                ],
            )?;
        }

        for block in &seed.blocks {
            transaction.execute(
                "INSERT INTO block(id, document_id, kind, order_key, content_json, plain_text, content_hash, revision, locked)
                 VALUES (?1, ?2, ?3, ?4, ?5, ?6, ?7, 0, ?8)",
                params![
                    block.id,
                    block.document_id,
                    block.kind,
                    block.order_key,
                    block.content_json,
                    block.plain_text,
                    block.content_hash,
                    i64::from(block.locked)
                ],
            )?;
        }

        transaction.execute(
            "INSERT INTO commit_node(id, project_id, root_hash, reason, actor_type, actor_id, created_at)
             VALUES (?1, ?2, ?3, 'initial', 'system', NULL, ?4)",
            params![seed.initial_commit_id, seed.project_id, seed.initial_root_hash, seed.created_at],
        )?;
        for (position, block) in seed.blocks.iter().enumerate() {
            transaction.execute(
                "INSERT INTO change_set(commit_id, position, entity_type, entity_id, operation, before_hash, after_hash, edit_journal_id)
                 VALUES (?1, ?2, 'block', ?3, 'create', NULL, ?4, NULL)",
                params![seed.initial_commit_id, position as i64, block.id, block.content_hash],
            )?;
        }
        enqueue_summary_invalidation(
            &transaction,
            &seed.project_id,
            "project",
            &seed.project_id,
            &seed.initial_commit_id,
            "initial",
            &seed.created_at,
        )?;
        for document in &seed.documents {
            enqueue_summary_invalidation(
                &transaction,
                &seed.project_id,
                "document",
                &document.id,
                &seed.initial_commit_id,
                "initial",
                &seed.created_at,
            )?;
        }
        for block in &seed.blocks {
            enqueue_summary_invalidation(
                &transaction,
                &seed.project_id,
                "block",
                &block.id,
                &seed.initial_commit_id,
                "initial",
                &seed.created_at,
            )?;
        }
        transaction.execute(
            "INSERT INTO branch(id, project_id, name, head_commit_id, created_at, updated_at)
             VALUES (?1, ?2, 'main', ?3, ?4, ?4)",
            params![
                seed.main_branch_id,
                seed.project_id,
                seed.initial_commit_id,
                seed.created_at
            ],
        )?;
        transaction.execute(
            "UPDATE project SET head_commit_id = ?2 WHERE id = ?1",
            params![seed.project_id, seed.initial_commit_id],
        )?;
        transaction.commit()?;
        Ok(())
    }

    pub fn get_project(&self, project_id: &str) -> StoreResult<ProjectRecord> {
        self.connection
            .query_row(
                "SELECT id, title, language, schema_version, head_commit_id, revision, created_at, updated_at
                 FROM project WHERE id = ?1",
                [project_id],
                |row| {
                    Ok(ProjectRecord {
                        id: row.get(0)?,
                        title: row.get(1)?,
                        language: row.get(2)?,
                        schema_version: row.get(3)?,
                        head_commit_id: row.get(4)?,
                        revision: row.get(5)?,
                        created_at: row.get(6)?,
                        updated_at: row.get(7)?,
                    })
                },
            )
            .optional()?
            .ok_or_else(|| StoreError::NotFound {
                entity: "project",
                id: project_id.to_owned(),
            })
    }

    pub fn get_branch(&self, branch_id: &str) -> StoreResult<BranchRecord> {
        self.connection
            .query_row(
                "SELECT id, project_id, name, head_commit_id, created_at, updated_at
                 FROM branch WHERE id = ?1",
                [branch_id],
                |row| {
                    Ok(BranchRecord {
                        id: row.get(0)?,
                        project_id: row.get(1)?,
                        name: row.get(2)?,
                        head_commit_id: row.get(3)?,
                        created_at: row.get(4)?,
                        updated_at: row.get(5)?,
                    })
                },
            )
            .optional()?
            .ok_or_else(|| StoreError::NotFound {
                entity: "branch",
                id: branch_id.to_owned(),
            })
    }

    pub fn list_style_samples(&self, project_id: &str) -> StoreResult<Vec<StyleSampleRecord>> {
        let mut statement = self.connection.prepare(
            "SELECT id, project_id, title, content, content_hash, status, sensitivity,
                    revision, created_at, updated_at
             FROM style_sample
             WHERE project_id = ?1
             ORDER BY status, updated_at DESC, id",
        )?;
        statement
            .query_map([project_id], map_style_sample)?
            .collect::<Result<_, _>>()
            .map_err(StoreError::from)
    }

    pub fn create_style_sample(
        &mut self,
        command: &CreateStyleSample,
    ) -> StoreResult<StyleSampleRecord> {
        validate_create_style_sample(command)?;
        let transaction = self
            .connection
            .transaction_with_behavior(TransactionBehavior::Immediate)?;
        let project_exists: bool = transaction.query_row(
            "SELECT EXISTS(SELECT 1 FROM project WHERE id = ?1)",
            [&command.project_id],
            |row| row.get(0),
        )?;
        if !project_exists {
            return Err(StoreError::NotFound {
                entity: "project",
                id: command.project_id.clone(),
            });
        }
        transaction.execute(
            "INSERT INTO style_sample(
               id, project_id, title, content, content_hash, status, sensitivity,
               revision, created_at, updated_at
             ) VALUES (?1, ?2, ?3, ?4, ?5, 'canonical', ?6, 0, ?7, ?7)",
            params![
                command.id,
                command.project_id,
                command.title,
                command.content,
                command.content_hash,
                command.sensitivity,
                command.created_at,
            ],
        )?;
        let result = read_style_sample(&transaction, &command.project_id, &command.id)?;
        transaction.commit()?;
        Ok(result)
    }

    pub fn set_style_sample_status(
        &mut self,
        command: &SetStyleSampleStatus,
    ) -> StoreResult<StyleSampleRecord> {
        validate_set_style_sample_status(command)?;
        let transaction = self
            .connection
            .transaction_with_behavior(TransactionBehavior::Immediate)?;
        let current = read_style_sample(&transaction, &command.project_id, &command.id)?;
        if current.revision != command.expected_revision {
            return Err(StoreError::StateConflict {
                entity: "style_sample",
                id: command.id.clone(),
                expected_revision: command.expected_revision,
                actual_revision: current.revision,
                expected_state: command.status.clone(),
                actual_state: current.status,
            });
        }
        if current.status == command.status {
            transaction.commit()?;
            return Ok(current);
        }
        let updated = transaction.execute(
            "UPDATE style_sample
             SET status = ?1, revision = revision + 1, updated_at = ?2
             WHERE project_id = ?3 AND id = ?4 AND revision = ?5",
            params![
                command.status,
                command.updated_at,
                command.project_id,
                command.id,
                command.expected_revision,
            ],
        )?;
        if updated != 1 {
            return Err(StoreError::InvariantViolation(
                "style sample status update changed an unexpected row count".into(),
            ));
        }
        let result = read_style_sample(&transaction, &command.project_id, &command.id)?;
        transaction.commit()?;
        Ok(result)
    }

    pub fn list_summary_invalidations(
        &self,
        project_id: &str,
        limit: usize,
    ) -> StoreResult<Vec<SummaryInvalidationRecord>> {
        if project_id.trim().is_empty() || limit == 0 || limit > 1_000 {
            return Err(StoreError::Validation(
                "project id and summary invalidation limit 1..=1000 are required".into(),
            ));
        }
        let mut statement = self.connection.prepare(
            "SELECT project_id, scope_type, scope_id, source_commit_id, reason,
                    invalidation_count, created_at
             FROM summary_invalidation
             WHERE project_id = ?1
             ORDER BY created_at, scope_type, scope_id
             LIMIT ?2",
        )?;
        statement
            .query_map(params![project_id, limit as i64], |row| {
                Ok(SummaryInvalidationRecord {
                    project_id: row.get(0)?,
                    scope_type: row.get(1)?,
                    scope_id: row.get(2)?,
                    source_commit_id: row.get(3)?,
                    reason: row.get(4)?,
                    invalidation_count: row.get(5)?,
                    created_at: row.get(6)?,
                })
            })?
            .collect::<Result<_, _>>()
            .map_err(StoreError::from)
    }

    pub fn put_summary_record(&mut self, command: &PutSummaryRecord) -> StoreResult<SummaryRecord> {
        validate_put_summary_record(command)?;
        let transaction = self
            .connection
            .transaction_with_behavior(TransactionBehavior::Immediate)?;
        validate_active_summary_scope(
            &transaction,
            &command.project_id,
            &command.scope_type,
            &command.scope_id,
        )?;
        let queued_commit: String = transaction
            .query_row(
                "SELECT source_commit_id FROM summary_invalidation
                 WHERE project_id = ?1 AND scope_type = ?2 AND scope_id = ?3",
                params![command.project_id, command.scope_type, command.scope_id],
                |row| row.get(0),
            )
            .optional()?
            .ok_or_else(|| StoreError::NotFound {
                entity: "summary_invalidation",
                id: format!("{}:{}", command.scope_type, command.scope_id),
            })?;
        if queued_commit != command.expected_source_commit_id {
            return Err(StoreError::StateConflict {
                entity: "summary_invalidation",
                id: format!("{}:{}", command.scope_type, command.scope_id),
                expected_revision: 0,
                actual_revision: 0,
                expected_state: command.expected_source_commit_id.clone(),
                actual_state: queued_commit,
            });
        }
        transaction.execute(
            "INSERT INTO summary_record(
               project_id, scope_type, scope_id, source_commit_id, source_hash, summary_text,
               summary_hash, provider_id, model, revision, generated_at, updated_at
             ) VALUES (?1, ?2, ?3, ?4, ?5, ?6, ?7, ?8, ?9, 0, ?10, ?10)
             ON CONFLICT(project_id, scope_type, scope_id) DO UPDATE SET
               source_commit_id = excluded.source_commit_id,
               source_hash = excluded.source_hash,
               summary_text = excluded.summary_text,
               summary_hash = excluded.summary_hash,
               provider_id = excluded.provider_id,
               model = excluded.model,
               revision = summary_record.revision + 1,
               generated_at = excluded.generated_at,
               updated_at = excluded.updated_at",
            params![
                command.project_id,
                command.scope_type,
                command.scope_id,
                command.expected_source_commit_id,
                command.source_hash,
                command.summary,
                command.summary_hash,
                command.provider_id,
                command.model,
                command.generated_at,
            ],
        )?;
        let removed = transaction.execute(
            "DELETE FROM summary_invalidation
             WHERE project_id = ?1 AND scope_type = ?2 AND scope_id = ?3
               AND source_commit_id = ?4",
            params![
                command.project_id,
                command.scope_type,
                command.scope_id,
                command.expected_source_commit_id,
            ],
        )?;
        if removed != 1 {
            return Err(StoreError::InvariantViolation(
                "summary completion did not consume exactly one invalidation".into(),
            ));
        }
        let record = read_summary_record(
            &transaction,
            &command.project_id,
            &command.scope_type,
            &command.scope_id,
        )?;
        transaction.commit()?;
        Ok(record)
    }

    pub fn create_document_with_block(
        &mut self,
        command: &CreateDocumentWithBlock,
    ) -> StoreResult<CreateDocumentReceipt> {
        validate_create_document(command)?;
        let transaction = self
            .connection
            .transaction_with_behavior(TransactionBehavior::Immediate)?;
        let (project_id, previous_head, project_revision): (String, String, i64) = transaction
            .query_row(
                "SELECT b.project_id, b.head_commit_id, p.revision
                 FROM branch AS b JOIN project AS p ON p.id = b.project_id
                 WHERE b.id = ?1",
                [&command.branch_id],
                |row| Ok((row.get(0)?, row.get(1)?, row.get(2)?)),
            )
            .optional()?
            .ok_or_else(|| StoreError::NotFound {
                entity: "branch",
                id: command.branch_id.clone(),
            })?;
        if previous_head != command.expected_head_commit_id
            || project_revision != command.expected_project_revision
        {
            return Err(StoreError::StateConflict {
                entity: "branch",
                id: command.branch_id.clone(),
                expected_revision: command.expected_project_revision,
                actual_revision: project_revision,
                expected_state: command.expected_head_commit_id.clone(),
                actual_state: previous_head,
            });
        }
        if let Some(parent_id) = &command.document_parent_id {
            let parent_exists: bool = transaction.query_row(
                "SELECT EXISTS(
                   SELECT 1 FROM document
                   WHERE id = ?1 AND project_id = ?2 AND deleted_at IS NULL
                 )",
                params![parent_id, project_id],
                |row| row.get(0),
            )?;
            if !parent_exists {
                return Err(StoreError::NotFound {
                    entity: "document",
                    id: parent_id.clone(),
                });
            }
        }
        let order_key_exists: bool = transaction.query_row(
            "SELECT EXISTS(
               SELECT 1 FROM document
               WHERE project_id = ?1 AND parent_id IS ?2 AND order_key = ?3
             )",
            params![
                project_id,
                command.document_parent_id,
                command.document_order_key
            ],
            |row| row.get(0),
        )?;
        if order_key_exists {
            return Err(StoreError::Validation(
                "document order key already exists within the parent".into(),
            ));
        }
        transaction.execute(
            "INSERT INTO document(id, project_id, parent_id, kind, title, order_key, revision)
             VALUES (?1, ?2, ?3, ?4, ?5, ?6, 0)",
            params![
                command.document_id,
                project_id,
                command.document_parent_id,
                command.document_kind,
                command.document_title,
                command.document_order_key,
            ],
        )?;
        transaction.execute(
            "INSERT INTO block(
               id, document_id, kind, order_key, content_json, plain_text,
               content_hash, revision, locked
             ) VALUES (?1, ?2, ?3, ?4, ?5, ?6, ?7, 0, ?8)",
            params![
                command.block_id,
                command.document_id,
                command.block_kind,
                command.block_order_key,
                command.block_content_json,
                command.block_plain_text,
                command.block_content_hash,
                i64::from(command.block_locked),
            ],
        )?;
        transaction.execute(
            "INSERT INTO commit_node(id, project_id, root_hash, reason, actor_type, actor_id, created_at)
             VALUES (?1, ?2, ?3, 'document_create', ?4, ?5, ?6)",
            params![
                command.commit_id,
                project_id,
                command.new_root_hash,
                command.actor_type,
                command.actor_id,
                command.occurred_at,
            ],
        )?;
        for (scope_type, scope_id) in [
            ("project", project_id.as_str()),
            ("document", command.document_id.as_str()),
            ("block", command.block_id.as_str()),
        ] {
            enqueue_summary_invalidation(
                &transaction,
                &project_id,
                scope_type,
                scope_id,
                &command.commit_id,
                "document_create",
                &command.occurred_at,
            )?;
        }
        transaction.execute(
            "INSERT INTO commit_parent(commit_id, parent_id, position) VALUES (?1, ?2, 0)",
            params![command.commit_id, previous_head],
        )?;
        transaction.execute(
            "INSERT INTO change_set(
               commit_id, position, entity_type, entity_id, operation,
               before_hash, after_hash, edit_journal_id
             ) VALUES (?1, 0, 'document', ?2, 'create', NULL, NULL, NULL)",
            params![command.commit_id, command.document_id],
        )?;
        transaction.execute(
            "INSERT INTO change_set(
               commit_id, position, entity_type, entity_id, operation,
               before_hash, after_hash, edit_journal_id
             ) VALUES (?1, 1, 'block', ?2, 'create', NULL, ?3, NULL)",
            params![
                command.commit_id,
                command.block_id,
                command.block_content_hash
            ],
        )?;
        let branch_updated = transaction.execute(
            "UPDATE branch SET head_commit_id = ?1, updated_at = ?2
             WHERE id = ?3 AND head_commit_id = ?4",
            params![
                command.commit_id,
                command.occurred_at,
                command.branch_id,
                previous_head,
            ],
        )?;
        if branch_updated != 1 {
            return Err(StoreError::InvariantViolation(
                "document creation branch update changed an unexpected row count".into(),
            ));
        }
        let next_revision = project_revision + 1;
        let project_updated = transaction.execute(
            "UPDATE project
             SET head_commit_id = ?1, revision = ?2, updated_at = ?3
             WHERE id = ?4 AND head_commit_id = ?5 AND revision = ?6",
            params![
                command.commit_id,
                next_revision,
                command.occurred_at,
                project_id,
                previous_head,
                project_revision,
            ],
        )?;
        if project_updated != 1 {
            return Err(StoreError::InvariantViolation(
                "document creation project update changed an unexpected row count".into(),
            ));
        }
        transaction.commit()?;
        Ok(CreateDocumentReceipt {
            document_id: command.document_id.clone(),
            block_id: command.block_id.clone(),
            commit_id: command.commit_id.clone(),
            previous_head_commit_id: previous_head,
            project_revision: next_revision,
        })
    }

    pub fn apply_document_batch(
        &mut self,
        command: &ApplyDocumentBatch,
    ) -> StoreResult<DocumentBatchReceipt> {
        validate_document_batch(command)?;
        let transaction = self
            .connection
            .transaction_with_behavior(TransactionBehavior::Immediate)?;
        let (project_id, previous_head, project_revision): (String, String, i64) = transaction
            .query_row(
                "SELECT b.project_id, b.head_commit_id, p.revision
                 FROM branch AS b JOIN project AS p ON p.id = b.project_id
                 WHERE b.id = ?1",
                [&command.branch_id],
                |row| Ok((row.get(0)?, row.get(1)?, row.get(2)?)),
            )
            .optional()?
            .ok_or_else(|| StoreError::NotFound {
                entity: "branch",
                id: command.branch_id.clone(),
            })?;
        if previous_head != command.expected_head_commit_id
            || project_revision != command.expected_project_revision
        {
            return Err(StoreError::StateConflict {
                entity: "branch",
                id: command.branch_id.clone(),
                expected_revision: command.expected_project_revision,
                actual_revision: project_revision,
                expected_state: command.expected_head_commit_id.clone(),
                actual_state: previous_head,
            });
        }

        let mut documents = {
            let mut statement = transaction.prepare(
                "SELECT id, project_id, parent_id, kind, title, order_key, revision, deleted_at
                 FROM document WHERE project_id = ?1 ORDER BY id",
            )?;
            statement
                .query_map([&project_id], map_document_record)?
                .collect::<Result<Vec<_>, _>>()?
        };
        let positions = documents
            .iter()
            .enumerate()
            .map(|(index, document)| (document.id.clone(), index))
            .collect::<BTreeMap<_, _>>();
        let mutation_ids = command
            .mutations
            .iter()
            .map(|mutation| mutation.document_id.as_str())
            .collect::<BTreeSet<_>>();
        let mut related_parent_ids = BTreeSet::new();
        for mutation in &command.mutations {
            let index =
                positions
                    .get(&mutation.document_id)
                    .ok_or_else(|| StoreError::NotFound {
                        entity: "document",
                        id: mutation.document_id.clone(),
                    })?;
            let document = &mut documents[*index];
            if document.revision != mutation.expected_revision {
                return Err(StoreError::StateConflict {
                    entity: "document",
                    id: document.id.clone(),
                    expected_revision: mutation.expected_revision,
                    actual_revision: document.revision,
                    expected_state: command.expected_head_commit_id.clone(),
                    actual_state: previous_head.clone(),
                });
            }
            if mutation.parent_id.as_deref() == Some(mutation.document_id.as_str()) {
                return Err(StoreError::Validation(
                    "a document cannot be its own parent".into(),
                ));
            }
            if let Some(parent_id) = document.parent_id.as_deref() {
                related_parent_ids.insert(parent_id.to_owned());
            }
            if let Some(parent_id) = mutation.parent_id.as_deref() {
                related_parent_ids.insert(parent_id.to_owned());
            }
            document.parent_id = mutation.parent_id.clone();
            document.kind = mutation.kind.clone();
            document.title = mutation.title.clone();
            document.order_key = mutation.order_key.clone();
            document.revision += 1;
            document.deleted_at = (!mutation.active).then(|| command.occurred_at.clone());
        }
        validate_final_document_tree(&documents)?;

        let occupied_order_keys = documents
            .iter()
            .map(|document| document.order_key.as_str())
            .collect::<BTreeSet<_>>();
        let temporary_order_keys = command
            .mutations
            .iter()
            .enumerate()
            .map(|(index, _)| format!("~{}-{index}", command.commit_id))
            .collect::<Vec<_>>();
        if temporary_order_keys
            .iter()
            .any(|key| occupied_order_keys.contains(key.as_str()))
        {
            return Err(StoreError::InvariantViolation(
                "document mutation temporary order key collided".into(),
            ));
        }
        for (mutation, temporary_order_key) in command.mutations.iter().zip(&temporary_order_keys) {
            let staged = transaction.execute(
                "UPDATE document SET parent_id = NULL, order_key = ?1
                 WHERE id = ?2 AND project_id = ?3 AND revision = ?4",
                params![
                    temporary_order_key,
                    mutation.document_id,
                    project_id,
                    mutation.expected_revision,
                ],
            )?;
            if staged != 1 {
                return Err(StoreError::InvariantViolation(
                    "document mutation staging changed an unexpected row count".into(),
                ));
            }
        }
        for mutation in &command.mutations {
            let deleted_at = (!mutation.active).then_some(command.occurred_at.as_str());
            let updated = transaction.execute(
                "UPDATE document
                 SET parent_id = ?1, kind = ?2, title = ?3, order_key = ?4,
                     revision = ?5, deleted_at = ?6
                 WHERE id = ?7 AND project_id = ?8 AND revision = ?9",
                params![
                    mutation.parent_id,
                    mutation.kind,
                    mutation.title,
                    mutation.order_key,
                    mutation.expected_revision + 1,
                    deleted_at,
                    mutation.document_id,
                    project_id,
                    mutation.expected_revision,
                ],
            )?;
            if updated != 1 {
                return Err(StoreError::InvariantViolation(
                    "document mutation changed an unexpected row count".into(),
                ));
            }
        }

        transaction.execute(
            "INSERT INTO commit_node(id, project_id, root_hash, reason, actor_type, actor_id, created_at)
             VALUES (?1, ?2, ?3, ?4, ?5, ?6, ?7)",
            params![
                command.commit_id,
                project_id,
                command.new_root_hash,
                command.reason,
                command.actor_type,
                command.actor_id,
                command.occurred_at,
            ],
        )?;
        enqueue_summary_invalidation(
            &transaction,
            &project_id,
            "project",
            &project_id,
            &command.commit_id,
            &command.reason,
            &command.occurred_at,
        )?;
        for mutation in &command.mutations {
            if mutation.active {
                enqueue_summary_invalidation(
                    &transaction,
                    &project_id,
                    "document",
                    &mutation.document_id,
                    &command.commit_id,
                    &command.reason,
                    &command.occurred_at,
                )?;
                if mutation.operation == "restore" {
                    let block_ids = {
                        let mut statement = transaction.prepare(
                            "SELECT b.id
                             FROM block AS b
                             JOIN document AS d ON d.id = b.document_id
                             WHERE d.project_id = ?1 AND d.id = ?2
                               AND d.deleted_at IS NULL AND b.deleted_at IS NULL
                             ORDER BY b.order_key, b.id",
                        )?;
                        statement
                            .query_map(params![project_id, mutation.document_id], |row| {
                                row.get::<_, String>(0)
                            })?
                            .collect::<Result<Vec<_>, _>>()?
                    };
                    for block_id in block_ids {
                        enqueue_summary_invalidation(
                            &transaction,
                            &project_id,
                            "block",
                            &block_id,
                            &command.commit_id,
                            &command.reason,
                            &command.occurred_at,
                        )?;
                    }
                }
            } else {
                transaction.execute(
                    "DELETE FROM summary_invalidation
                     WHERE project_id = ?1 AND (
                       (scope_type = 'document' AND scope_id = ?2)
                       OR (scope_type = 'block' AND scope_id IN (
                         SELECT id FROM block WHERE document_id = ?2
                       ))
                     )",
                    params![project_id, mutation.document_id],
                )?;
            }
        }
        for parent_id in related_parent_ids {
            if mutation_ids.contains(parent_id.as_str()) {
                continue;
            }
            let active = documents
                .iter()
                .any(|document| document.id == parent_id && document.deleted_at.is_none());
            if active {
                enqueue_summary_invalidation(
                    &transaction,
                    &project_id,
                    "document",
                    &parent_id,
                    &command.commit_id,
                    &command.reason,
                    &command.occurred_at,
                )?;
            }
        }
        transaction.execute(
            "INSERT INTO commit_parent(commit_id, parent_id, position) VALUES (?1, ?2, 0)",
            params![command.commit_id, previous_head],
        )?;
        for (position, mutation) in command.mutations.iter().enumerate() {
            transaction.execute(
                "INSERT INTO change_set(
                   commit_id, position, entity_type, entity_id, operation,
                   before_hash, after_hash, edit_journal_id
                 ) VALUES (?1, ?2, 'document', ?3, ?4, ?5, ?6, NULL)",
                params![
                    command.commit_id,
                    position as i64,
                    mutation.document_id,
                    mutation.operation,
                    mutation.before_hash,
                    mutation.after_hash,
                ],
            )?;
        }
        let branch_updated = transaction.execute(
            "UPDATE branch SET head_commit_id = ?1, updated_at = ?2
             WHERE id = ?3 AND head_commit_id = ?4",
            params![
                command.commit_id,
                command.occurred_at,
                command.branch_id,
                previous_head,
            ],
        )?;
        if branch_updated != 1 {
            return Err(StoreError::InvariantViolation(
                "document mutation branch update changed an unexpected row count".into(),
            ));
        }
        let next_revision = project_revision + 1;
        let project_updated = transaction.execute(
            "UPDATE project
             SET head_commit_id = ?1, revision = ?2, updated_at = ?3
             WHERE id = ?4 AND head_commit_id = ?5 AND revision = ?6",
            params![
                command.commit_id,
                next_revision,
                command.occurred_at,
                project_id,
                previous_head,
                project_revision,
            ],
        )?;
        if project_updated != 1 {
            return Err(StoreError::InvariantViolation(
                "document mutation project update changed an unexpected row count".into(),
            ));
        }
        transaction.commit()?;
        Ok(DocumentBatchReceipt {
            commit_id: command.commit_id.clone(),
            previous_head_commit_id: previous_head,
            project_revision: next_revision,
        })
    }

    pub fn list_documents(&self, project_id: &str) -> StoreResult<Vec<DocumentRecord>> {
        let mut statement = self.connection.prepare(
            "SELECT id, project_id, parent_id, kind, title, order_key, revision, deleted_at
             FROM document
             WHERE project_id = ?1 AND deleted_at IS NULL
             ORDER BY order_key, id",
        )?;
        statement
            .query_map([project_id], |row| {
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
            })?
            .collect::<Result<_, _>>()
            .map_err(StoreError::from)
    }

    pub fn list_archived_documents(&self, project_id: &str) -> StoreResult<Vec<DocumentRecord>> {
        let mut statement = self.connection.prepare(
            "SELECT id, project_id, parent_id, kind, title, order_key, revision, deleted_at
             FROM document
             WHERE project_id = ?1 AND deleted_at IS NOT NULL
             ORDER BY deleted_at DESC, id",
        )?;
        statement
            .query_map([project_id], map_document_record)?
            .collect::<Result<_, _>>()
            .map_err(StoreError::from)
    }

    pub fn get_document(&self, project_id: &str, document_id: &str) -> StoreResult<DocumentRecord> {
        self.connection
            .query_row(
                "SELECT id, project_id, parent_id, kind, title, order_key, revision, deleted_at
                 FROM document WHERE project_id = ?1 AND id = ?2",
                params![project_id, document_id],
                map_document_record,
            )
            .optional()?
            .ok_or_else(|| StoreError::NotFound {
                entity: "document",
                id: document_id.to_owned(),
            })
    }

    pub fn list_blocks(&self, project_id: &str) -> StoreResult<Vec<BlockRecord>> {
        let mut statement = self.connection.prepare(
            "SELECT b.id, b.document_id, b.kind, b.order_key, b.content_json, b.plain_text,
                    b.content_hash, b.revision, b.locked
             FROM block AS b
             JOIN document AS d ON d.id = b.document_id
             WHERE d.project_id = ?1 AND d.deleted_at IS NULL AND b.deleted_at IS NULL
             ORDER BY d.order_key, d.id, b.order_key, b.id",
        )?;
        statement
            .query_map([project_id], map_block_record)?
            .collect::<Result<_, _>>()
            .map_err(StoreError::from)
    }

    pub fn list_all_blocks(&self, project_id: &str) -> StoreResult<Vec<BlockRecord>> {
        let mut statement = self.connection.prepare(
            "SELECT b.id, b.document_id, b.kind, b.order_key, b.content_json, b.plain_text,
                    b.content_hash, b.revision, b.locked
             FROM block AS b
             JOIN document AS d ON d.id = b.document_id
             WHERE d.project_id = ?1 AND b.deleted_at IS NULL
             ORDER BY d.id, b.order_key, b.id",
        )?;
        statement
            .query_map([project_id], map_block_record)?
            .collect::<Result<_, _>>()
            .map_err(StoreError::from)
    }

    pub fn get_project_block(&self, project_id: &str, block_id: &str) -> StoreResult<BlockRecord> {
        self.connection
            .query_row(
                "SELECT b.id, b.document_id, b.kind, b.order_key, b.content_json, b.plain_text,
                        b.content_hash, b.revision, b.locked
                 FROM block AS b
                 JOIN document AS d ON d.id = b.document_id
                 WHERE d.project_id = ?1 AND b.id = ?2
                   AND d.deleted_at IS NULL AND b.deleted_at IS NULL",
                params![project_id, block_id],
                map_block_record,
            )
            .optional()?
            .ok_or_else(|| StoreError::NotFound {
                entity: "block",
                id: block_id.to_owned(),
            })
    }

    pub fn get_block(&self, block_id: &str) -> StoreResult<BlockRecord> {
        self.connection
            .query_row(
                "SELECT id, document_id, kind, order_key, content_json, plain_text, content_hash, revision, locked
                 FROM block WHERE id = ?1 AND deleted_at IS NULL",
                [block_id],
                map_block_record,
            )
            .optional()?
            .ok_or_else(|| StoreError::NotFound { entity: "block", id: block_id.to_owned() })
    }

    pub fn apply_block_edit(&mut self, command: &ApplyBlockEdit) -> StoreResult<EditReceipt> {
        validate_edit(command)?;
        let transaction = self
            .connection
            .transaction_with_behavior(TransactionBehavior::Immediate)?;
        let current = read_block_for_edit(&transaction, &command.block_id)?;
        if current.locked {
            return Err(StoreError::Validation(format!(
                "block {} is locked",
                command.block_id
            )));
        }
        if current.revision != command.expected_revision
            || current.content_hash != command.expected_hash
        {
            return Err(StoreError::Conflict {
                entity: "block",
                id: command.block_id.clone(),
                expected_revision: command.expected_revision,
                actual_revision: current.revision,
                expected_hash: command.expected_hash.clone(),
                actual_hash: current.content_hash,
            });
        }

        let (branch_project_id, previous_head, project_revision): (String, String, i64) =
            transaction
                .query_row(
                    "SELECT b.project_id, b.head_commit_id, p.revision
                 FROM branch AS b
                 JOIN project AS p ON p.id = b.project_id
                 WHERE b.id = ?1",
                    [&command.branch_id],
                    |row| Ok((row.get(0)?, row.get(1)?, row.get(2)?)),
                )
                .optional()?
                .ok_or_else(|| StoreError::NotFound {
                    entity: "branch",
                    id: command.branch_id.clone(),
                })?;
        if branch_project_id != current.project_id {
            return Err(StoreError::Validation(
                "branch and block belong to different projects".into(),
            ));
        }
        if previous_head != command.expected_head_commit_id
            || project_revision != command.expected_project_revision
        {
            return Err(StoreError::StateConflict {
                entity: "branch",
                id: command.branch_id.clone(),
                expected_revision: command.expected_project_revision,
                actual_revision: project_revision,
                expected_state: command.expected_head_commit_id.clone(),
                actual_state: previous_head,
            });
        }

        let new_revision = current.revision + 1;
        let updated = transaction.execute(
            "UPDATE block
             SET content_json = ?1, plain_text = ?2, content_hash = ?3, revision = ?4
             WHERE id = ?5 AND revision = ?6 AND content_hash = ?7 AND locked = 0 AND deleted_at IS NULL",
            params![
                command.new_content_json,
                command.new_plain_text,
                command.new_content_hash,
                new_revision,
                command.block_id,
                command.expected_revision,
                command.expected_hash
            ],
        )?;
        if updated != 1 {
            return Err(StoreError::InvariantViolation(
                "optimistic block update changed an unexpected row count".into(),
            ));
        }

        transaction.execute(
            "INSERT INTO edit_journal(
               id, project_id, block_id, base_revision, new_revision, before_hash, after_hash,
               before_content_json, after_content_json, before_plain_text, after_plain_text, occurred_at
             ) VALUES (?1, ?2, ?3, ?4, ?5, ?6, ?7, ?8, ?9, ?10, ?11, ?12)",
            params![
                command.edit_id,
                current.project_id,
                command.block_id,
                current.revision,
                new_revision,
                current.content_hash,
                command.new_content_hash,
                current.content_json,
                command.new_content_json,
                current.plain_text,
                command.new_plain_text,
                command.occurred_at
            ],
        )?;
        let document_updated = transaction.execute(
            "UPDATE document SET revision = revision + 1
             WHERE id = ?1 AND deleted_at IS NULL",
            [&current.document_id],
        )?;
        if document_updated != 1 {
            return Err(StoreError::InvariantViolation(
                "document revision update changed an unexpected row count".into(),
            ));
        }
        transaction.execute(
            "INSERT INTO commit_node(id, project_id, root_hash, reason, actor_type, actor_id, created_at)
             VALUES (?1, ?2, ?3, ?4, ?5, ?6, ?7)",
            params![
                command.commit_id,
                current.project_id,
                command.new_root_hash,
                command.reason,
                command.actor_type,
                command.actor_id,
                command.occurred_at
            ],
        )?;
        for (scope_type, scope_id) in [
            ("project", current.project_id.as_str()),
            ("document", current.document_id.as_str()),
            ("block", command.block_id.as_str()),
        ] {
            enqueue_summary_invalidation(
                &transaction,
                &current.project_id,
                scope_type,
                scope_id,
                &command.commit_id,
                &command.reason,
                &command.occurred_at,
            )?;
        }
        transaction.execute(
            "INSERT INTO commit_parent(commit_id, parent_id, position) VALUES (?1, ?2, 0)",
            params![command.commit_id, previous_head],
        )?;
        transaction.execute(
            "INSERT INTO change_set(commit_id, position, entity_type, entity_id, operation, before_hash, after_hash, edit_journal_id)
             VALUES (?1, 0, 'block', ?2, 'update', ?3, ?4, ?5)",
            params![command.commit_id, command.block_id, command.expected_hash, command.new_content_hash, command.edit_id],
        )?;
        let branch_updated = transaction.execute(
            "UPDATE branch SET head_commit_id = ?1, updated_at = ?2 WHERE id = ?3 AND head_commit_id = ?4",
            params![command.commit_id, command.occurred_at, command.branch_id, previous_head],
        )?;
        if branch_updated != 1 {
            return Err(StoreError::InvariantViolation(
                "branch head changed while applying an edit".into(),
            ));
        }
        let project_updated = transaction.execute(
            "UPDATE project SET head_commit_id = ?1, revision = revision + 1, updated_at = ?2 WHERE id = ?3",
            params![command.commit_id, command.occurred_at, current.project_id],
        )?;
        if project_updated != 1 {
            return Err(StoreError::InvariantViolation(
                "project head update changed an unexpected row count".into(),
            ));
        }
        if let Some(review_event) = &command.review_event {
            if review_event.kind != ReviewEventKind::Apply
                || review_event.expected_status != ReviewSessionStatus::Ready
                || review_event.next_status != ReviewSessionStatus::Applied
                || review_event.occurred_at != command.occurred_at
                || command.reason != "ai_accept"
                || command.actor_type != "model"
            {
                return Err(StoreError::Validation(
                    "reviewed edit must atomically apply one ready proposal as a model ai_accept commit"
                        .into(),
                ));
            }
            append_review_event_in_transaction(&transaction, review_event)?;
        }
        transaction.commit()?;

        Ok(EditReceipt {
            edit_id: command.edit_id.clone(),
            commit_id: command.commit_id.clone(),
            previous_head_commit_id: previous_head,
            block_id: command.block_id.clone(),
            new_revision,
            new_content_hash: command.new_content_hash.clone(),
        })
    }

    pub fn list_commits(&self, project_id: &str) -> StoreResult<Vec<CommitRecord>> {
        let mut statement = self.connection.prepare(
            "SELECT id, project_id, root_hash, reason, actor_type, actor_id, created_at
             FROM commit_node WHERE project_id = ?1 ORDER BY created_at, id",
        )?;
        let rows = statement.query_map([project_id], |row| {
            Ok(CommitRecord {
                id: row.get(0)?,
                project_id: row.get(1)?,
                root_hash: row.get(2)?,
                reason: row.get(3)?,
                actor_type: row.get(4)?,
                actor_id: row.get(5)?,
                created_at: row.get(6)?,
                parents: Vec::new(),
            })
        })?;
        let mut commits: Vec<CommitRecord> = rows.collect::<Result<_, _>>()?;
        drop(statement);
        for commit in &mut commits {
            let mut parents = self.connection.prepare(
                "SELECT parent_id FROM commit_parent WHERE commit_id = ?1 ORDER BY position",
            )?;
            commit.parents = parents
                .query_map([&commit.id], |row| row.get(0))?
                .collect::<Result<_, _>>()?;
        }
        Ok(commits)
    }

    pub fn create_snapshot(&mut self, snapshot: &CreateSnapshot) -> StoreResult<()> {
        if snapshot.codec.trim().is_empty()
            || snapshot.checksum.trim().is_empty()
            || snapshot.codec_version < 1
        {
            return Err(StoreError::Validation(
                "snapshot codec, checksum and codec version are required".into(),
            ));
        }
        let transaction = self
            .connection
            .transaction_with_behavior(TransactionBehavior::Immediate)?;
        let actual_root: String = transaction
            .query_row(
                "SELECT root_hash FROM commit_node WHERE id = ?1 AND project_id = ?2",
                params![snapshot.commit_id, snapshot.project_id],
                |row| row.get(0),
            )
            .optional()?
            .ok_or_else(|| StoreError::NotFound {
                entity: "commit",
                id: snapshot.commit_id.clone(),
            })?;
        if actual_root != snapshot.root_hash {
            return Err(StoreError::Validation(
                "snapshot root hash does not match commit root hash".into(),
            ));
        }
        transaction.execute(
            "INSERT INTO materialized_snapshot(
               id, project_id, commit_id, root_hash, codec, codec_version, payload, checksum, created_at
             ) VALUES (?1, ?2, ?3, ?4, ?5, ?6, ?7, ?8, ?9)",
            params![
                snapshot.id,
                snapshot.project_id,
                snapshot.commit_id,
                snapshot.root_hash,
                snapshot.codec,
                snapshot.codec_version,
                snapshot.payload,
                snapshot.checksum,
                snapshot.created_at
            ],
        )?;
        transaction.commit()?;
        Ok(())
    }

    pub fn latest_snapshot(&self, project_id: &str) -> StoreResult<Option<SnapshotRecord>> {
        self.connection
            .query_row(
                "SELECT id, project_id, commit_id, root_hash, codec, codec_version, payload, checksum, created_at
                 FROM materialized_snapshot WHERE project_id = ?1 ORDER BY created_at DESC, id DESC LIMIT 1",
                [project_id],
                |row| {
                    Ok(SnapshotRecord {
                        id: row.get(0)?,
                        project_id: row.get(1)?,
                        commit_id: row.get(2)?,
                        root_hash: row.get(3)?,
                        codec: row.get(4)?,
                        codec_version: row.get(5)?,
                        payload: row.get(6)?,
                        checksum: row.get(7)?,
                        created_at: row.get(8)?,
                    })
                },
            )
            .optional()
            .map_err(StoreError::from)
    }

    pub fn list_snapshots(&self, project_id: &str) -> StoreResult<Vec<SnapshotRecord>> {
        let mut statement = self.connection.prepare(
            "SELECT id, project_id, commit_id, root_hash, codec, codec_version, payload,
                    checksum, created_at
             FROM materialized_snapshot
             WHERE project_id = ?1
             ORDER BY created_at DESC, id DESC",
        )?;
        statement
            .query_map([project_id], |row| {
                Ok(SnapshotRecord {
                    id: row.get(0)?,
                    project_id: row.get(1)?,
                    commit_id: row.get(2)?,
                    root_hash: row.get(3)?,
                    codec: row.get(4)?,
                    codec_version: row.get(5)?,
                    payload: row.get(6)?,
                    checksum: row.get(7)?,
                    created_at: row.get(8)?,
                })
            })?
            .collect::<Result<_, _>>()
            .map_err(StoreError::from)
    }

    pub fn get_snapshot(&self, snapshot_id: &str) -> StoreResult<SnapshotRecord> {
        self.connection
            .query_row(
                "SELECT id, project_id, commit_id, root_hash, codec, codec_version, payload, checksum, created_at
                 FROM materialized_snapshot WHERE id = ?1",
                [snapshot_id],
                |row| {
                    Ok(SnapshotRecord {
                        id: row.get(0)?,
                        project_id: row.get(1)?,
                        commit_id: row.get(2)?,
                        root_hash: row.get(3)?,
                        codec: row.get(4)?,
                        codec_version: row.get(5)?,
                        payload: row.get(6)?,
                        checksum: row.get(7)?,
                        created_at: row.get(8)?,
                    })
                },
            )
            .optional()?
            .ok_or_else(|| StoreError::NotFound {
                entity: "snapshot",
                id: snapshot_id.to_owned(),
            })
    }

    pub fn decode_snapshot_record(
        &self,
        record: &SnapshotRecord,
    ) -> StoreResult<ProjectSnapshotV1> {
        if record.codec != SNAPSHOT_CODEC || record.codec_version != SNAPSHOT_CODEC_VERSION {
            return Err(StoreError::Snapshot(format!(
                "unsupported codec {}/{}",
                record.codec, record.codec_version
            )));
        }
        let snapshot = decode_snapshot(&record.payload, &record.checksum)
            .map_err(|error| StoreError::Snapshot(error.to_string()))?;
        if snapshot.project.id != record.project_id
            || snapshot.commit.id != record.commit_id
            || snapshot.commit.root_hash != record.root_hash
        {
            return Err(StoreError::Snapshot(
                "payload descriptor does not match snapshot database record".into(),
            ));
        }
        Ok(snapshot)
    }

    pub fn create_head_snapshot(
        &mut self,
        snapshot_id: &str,
        project_id: &str,
        created_at: &str,
    ) -> StoreResult<SnapshotRecord> {
        if snapshot_id.trim().is_empty()
            || project_id.trim().is_empty()
            || created_at.trim().is_empty()
        {
            return Err(StoreError::Validation(
                "snapshot id, project id and created_at are required".into(),
            ));
        }
        let transaction = self
            .connection
            .transaction_with_behavior(TransactionBehavior::Immediate)?;
        let snapshot = read_head_snapshot(&transaction, project_id)?;
        let encoded =
            encode_snapshot(&snapshot).map_err(|error| StoreError::Snapshot(error.to_string()))?;
        let record = SnapshotRecord {
            id: snapshot_id.to_owned(),
            project_id: snapshot.project.id.clone(),
            commit_id: snapshot.commit.id.clone(),
            root_hash: snapshot.commit.root_hash.clone(),
            codec: SNAPSHOT_CODEC.into(),
            codec_version: SNAPSHOT_CODEC_VERSION,
            payload: encoded.payload,
            checksum: encoded.checksum,
            created_at: created_at.to_owned(),
        };
        transaction.execute(
            "INSERT INTO materialized_snapshot(
               id, project_id, commit_id, root_hash, codec, codec_version, payload, checksum, created_at
             ) VALUES (?1, ?2, ?3, ?4, ?5, ?6, ?7, ?8, ?9)",
            params![
                record.id,
                record.project_id,
                record.commit_id,
                record.root_hash,
                record.codec,
                record.codec_version,
                record.payload,
                record.checksum,
                record.created_at
            ],
        )?;
        transaction.commit()?;
        Ok(record)
    }

    pub fn restore_snapshot(&mut self, command: &RestoreSnapshot) -> StoreResult<RestoreReceipt> {
        validate_restore(command)?;
        let record = self.get_snapshot(&command.snapshot_id)?;
        let snapshot = self.decode_snapshot_record(&record)?;
        let transaction = self
            .connection
            .transaction_with_behavior(TransactionBehavior::Immediate)?;

        let (branch_project_id, previous_head): (String, String) = transaction
            .query_row(
                "SELECT project_id, head_commit_id FROM branch WHERE id = ?1",
                [&command.branch_id],
                |row| Ok((row.get(0)?, row.get(1)?)),
            )
            .optional()?
            .ok_or_else(|| StoreError::NotFound {
                entity: "branch",
                id: command.branch_id.clone(),
            })?;
        if branch_project_id != snapshot.project.id {
            return Err(StoreError::Validation(
                "snapshot and branch belong to different projects".into(),
            ));
        }

        struct RestoredChange {
            entity_type: &'static str,
            entity_id: String,
            operation: &'static str,
            before_hash: Option<String>,
            after_hash: Option<String>,
            edit_id: Option<String>,
        }
        let mut changes = Vec::new();
        let mut changed_documents = BTreeSet::new();
        let mut revised_documents = BTreeSet::new();
        let current_documents = read_active_documents(&transaction, &branch_project_id)?;
        let current_blocks = read_active_snapshot_blocks(&transaction, &branch_project_id)?;
        let current_document_map: BTreeMap<_, _> = current_documents
            .iter()
            .map(|document| (document.id.as_str(), document))
            .collect();
        let snapshot_document_map: BTreeMap<_, _> = snapshot
            .documents
            .iter()
            .map(|document| (document.id.as_str(), document))
            .collect();
        let current_block_map: BTreeMap<_, _> = current_blocks
            .iter()
            .map(|block| (block.id.as_str(), block))
            .collect();
        let snapshot_block_map: BTreeMap<_, _> = snapshot
            .blocks
            .iter()
            .map(|block| (block.id.as_str(), block))
            .collect();

        for (id, target) in &snapshot_block_map {
            let Some(current) = current_block_map.get(id) else {
                continue;
            };
            if current.document_id != target.document_id
                || current.kind != target.kind
                || current.order_key != target.order_key
                || current.locked != target.locked
            {
                return Err(StoreError::Validation(format!(
                    "block metadata changed and cannot be structurally restored: {id}"
                )));
            }
        }

        let document_structure_changed = current_documents.len() != snapshot.documents.len()
            || snapshot.documents.iter().any(|target| {
                current_document_map
                    .get(target.id.as_str())
                    .is_none_or(|current| {
                        current.parent_id != target.parent_id
                            || current.kind != target.kind
                            || current.title != target.title
                            || current.order_key != target.order_key
                    })
            });
        if document_structure_changed {
            for (position, document) in current_documents.iter().enumerate() {
                let temporary_order_key = format!("~restore-{}-{position}", command.new_commit_id);
                let staged = transaction.execute(
                    "UPDATE document SET parent_id = NULL, order_key = ?1
                     WHERE id = ?2 AND project_id = ?3 AND revision = ?4
                       AND deleted_at IS NULL",
                    params![
                        temporary_order_key,
                        document.id,
                        branch_project_id,
                        document.revision,
                    ],
                )?;
                if staged != 1 {
                    return Err(StoreError::InvariantViolation(
                        "structural restore document staging changed an unexpected row count"
                            .into(),
                    ));
                }
            }
        }

        for block in current_blocks
            .iter()
            .filter(|block| !snapshot_block_map.contains_key(block.id.as_str()))
        {
            let updated = transaction.execute(
                "UPDATE block SET deleted_at = ?1
                 WHERE id = ?2 AND deleted_at IS NULL",
                params![command.occurred_at, block.id],
            )?;
            if updated != 1 {
                return Err(StoreError::InvariantViolation(
                    "structural restore could not archive a block".into(),
                ));
            }
            changes.push(RestoredChange {
                entity_type: "block",
                entity_id: block.id.clone(),
                operation: "archive",
                before_hash: Some(block.content_hash.clone()),
                after_hash: None,
                edit_id: None,
            });
            changed_documents.insert(block.document_id.clone());
        }
        for (position, document) in current_documents
            .iter()
            .filter(|document| !snapshot_document_map.contains_key(document.id.as_str()))
            .enumerate()
        {
            let archived_order_key = format!("~archived-{}-{position}", command.new_commit_id);
            let updated = transaction.execute(
                "UPDATE document
                 SET parent_id = NULL, order_key = ?1, deleted_at = ?2, revision = revision + 1
                 WHERE id = ?3 AND project_id = ?4 AND deleted_at IS NULL",
                params![
                    archived_order_key,
                    command.occurred_at,
                    document.id,
                    branch_project_id,
                ],
            )?;
            if updated != 1 {
                return Err(StoreError::InvariantViolation(
                    "structural restore could not archive a document".into(),
                ));
            }
            revised_documents.insert(document.id.clone());
            changes.push(RestoredChange {
                entity_type: "document",
                entity_id: document.id.clone(),
                operation: "archive",
                before_hash: None,
                after_hash: None,
                edit_id: None,
            });
        }

        for target in &snapshot.documents {
            let Some(current) = current_document_map.get(target.id.as_str()) else {
                continue;
            };
            let metadata_changed = current.parent_id != target.parent_id
                || current.kind != target.kind
                || current.title != target.title
                || current.order_key != target.order_key;
            let next_revision = current.revision + i64::from(metadata_changed);
            let updated = transaction.execute(
                "UPDATE document
                 SET parent_id = ?1, kind = ?2, title = ?3, order_key = ?4, revision = ?5
                 WHERE id = ?6 AND project_id = ?7 AND revision = ?8
                   AND deleted_at IS NULL",
                params![
                    target.parent_id,
                    target.kind,
                    target.title,
                    target.order_key,
                    next_revision,
                    target.id,
                    branch_project_id,
                    current.revision,
                ],
            )?;
            if updated != 1 {
                return Err(StoreError::InvariantViolation(
                    "structural restore document update changed an unexpected row count".into(),
                ));
            }
            if metadata_changed {
                revised_documents.insert(target.id.clone());
                changes.push(RestoredChange {
                    entity_type: "document",
                    entity_id: target.id.clone(),
                    operation: "restore",
                    before_hash: None,
                    after_hash: None,
                    edit_id: None,
                });
            }
        }

        for document in snapshot
            .documents
            .iter()
            .filter(|document| !current_document_map.contains_key(document.id.as_str()))
        {
            let existing: Option<(String, i64)> = transaction
                .query_row(
                    "SELECT project_id, revision FROM document WHERE id = ?1",
                    [&document.id],
                    |row| Ok((row.get(0)?, row.get(1)?)),
                )
                .optional()?;
            match existing {
                Some((existing_project_id, revision)) => {
                    if existing_project_id != branch_project_id {
                        return Err(StoreError::Validation(
                            "snapshot document id belongs to another project".into(),
                        ));
                    }
                    transaction.execute(
                        "UPDATE document
                         SET parent_id = ?1, kind = ?2, title = ?3, order_key = ?4,
                             revision = ?5, deleted_at = NULL
                         WHERE id = ?6",
                        params![
                            document.parent_id,
                            document.kind,
                            document.title,
                            document.order_key,
                            revision + 1,
                            document.id,
                        ],
                    )?;
                }
                None => {
                    transaction.execute(
                        "INSERT INTO document(
                           id, project_id, parent_id, kind, title, order_key, revision, deleted_at
                         ) VALUES (?1, ?2, ?3, ?4, ?5, ?6, ?7, NULL)",
                        params![
                            document.id,
                            branch_project_id,
                            document.parent_id,
                            document.kind,
                            document.title,
                            document.order_key,
                            document.revision,
                        ],
                    )?;
                }
            }
            revised_documents.insert(document.id.clone());
            changes.push(RestoredChange {
                entity_type: "document",
                entity_id: document.id.clone(),
                operation: "restore",
                before_hash: None,
                after_hash: None,
                edit_id: None,
            });
        }

        for block in snapshot
            .blocks
            .iter()
            .filter(|block| !current_block_map.contains_key(block.id.as_str()))
        {
            let existing: Option<i64> = transaction
                .query_row(
                    "SELECT revision FROM block WHERE id = ?1",
                    [&block.id],
                    |row| row.get(0),
                )
                .optional()?;
            match existing {
                Some(revision) => {
                    transaction.execute(
                        "UPDATE block
                         SET document_id = ?1, kind = ?2, order_key = ?3, content_json = ?4,
                             plain_text = ?5, content_hash = ?6, revision = ?7, locked = ?8,
                             deleted_at = NULL
                         WHERE id = ?9",
                        params![
                            block.document_id,
                            block.kind,
                            block.order_key,
                            block.content_json,
                            block.plain_text,
                            block.content_hash,
                            revision,
                            i64::from(block.locked),
                            block.id,
                        ],
                    )?;
                }
                None => {
                    transaction.execute(
                        "INSERT INTO block(
                           id, document_id, kind, order_key, content_json, plain_text,
                           content_hash, revision, locked, deleted_at
                         ) VALUES (?1, ?2, ?3, ?4, ?5, ?6, ?7, ?8, ?9, NULL)",
                        params![
                            block.id,
                            block.document_id,
                            block.kind,
                            block.order_key,
                            block.content_json,
                            block.plain_text,
                            block.content_hash,
                            block.revision,
                            i64::from(block.locked),
                        ],
                    )?;
                }
            }
            changes.push(RestoredChange {
                entity_type: "block",
                entity_id: block.id.clone(),
                operation: "restore",
                before_hash: None,
                after_hash: Some(block.content_hash.clone()),
                edit_id: None,
            });
            changed_documents.insert(block.document_id.clone());
        }

        for block in &snapshot.blocks {
            if !current_block_map.contains_key(block.id.as_str()) {
                continue;
            }
            let current = read_block_for_edit(&transaction, &block.id)?;
            if current.content_hash == block.content_hash
                && current.content_json == block.content_json
                && current.plain_text == block.plain_text
            {
                continue;
            }
            let position = changes.len();
            let edit_id = format!("{}-{position}", command.edit_id_prefix);
            let new_revision = current.revision + 1;
            let updated = transaction.execute(
                "UPDATE block SET content_json = ?1, plain_text = ?2, content_hash = ?3, revision = ?4
                 WHERE id = ?5 AND revision = ?6 AND content_hash = ?7 AND deleted_at IS NULL",
                params![
                    block.content_json,
                    block.plain_text,
                    block.content_hash,
                    new_revision,
                    block.id,
                    current.revision,
                    current.content_hash
                ],
            )?;
            if updated != 1 {
                return Err(StoreError::InvariantViolation(
                    "restore optimistic block update changed an unexpected row count".into(),
                ));
            }
            transaction.execute(
                "INSERT INTO edit_journal(
                   id, project_id, block_id, base_revision, new_revision, before_hash, after_hash,
                   before_content_json, after_content_json, before_plain_text, after_plain_text, occurred_at
                 ) VALUES (?1, ?2, ?3, ?4, ?5, ?6, ?7, ?8, ?9, ?10, ?11, ?12)",
                params![
                    edit_id,
                    branch_project_id,
                    block.id,
                    current.revision,
                    new_revision,
                    current.content_hash,
                    block.content_hash,
                    current.content_json,
                    block.content_json,
                    current.plain_text,
                    block.plain_text,
                    command.occurred_at
                ],
            )?;
            changes.push(RestoredChange {
                entity_type: "block",
                entity_id: block.id.clone(),
                operation: "restore",
                before_hash: Some(current.content_hash),
                after_hash: Some(block.content_hash.clone()),
                edit_id: Some(edit_id),
            });
            changed_documents.insert(current.document_id);
        }
        if changes.is_empty() {
            return Err(StoreError::Validation(
                "snapshot already matches the current project state".into(),
            ));
        }
        for document_id in changed_documents.difference(&revised_documents) {
            let updated = transaction.execute(
                "UPDATE document SET revision = revision + 1
                 WHERE id = ?1 AND deleted_at IS NULL",
                [document_id],
            )?;
            if updated > 1 {
                return Err(StoreError::InvariantViolation(
                    "restore document revision update changed an unexpected row count".into(),
                ));
            }
        }

        transaction.execute(
            "INSERT INTO commit_node(id, project_id, root_hash, reason, actor_type, actor_id, created_at)
             VALUES (?1, ?2, ?3, 'restore', ?4, ?5, ?6)",
            params![
                command.new_commit_id,
                branch_project_id,
                snapshot.commit.root_hash,
                command.actor_type,
                command.actor_id,
                command.occurred_at
            ],
        )?;
        enqueue_summary_invalidation(
            &transaction,
            &branch_project_id,
            "project",
            &branch_project_id,
            &command.new_commit_id,
            "restore",
            &command.occurred_at,
        )?;
        for document_id in changed_documents.union(&revised_documents) {
            let active: bool = transaction.query_row(
                "SELECT EXISTS(
                   SELECT 1 FROM document
                   WHERE project_id = ?1 AND id = ?2 AND deleted_at IS NULL
                 )",
                params![branch_project_id, document_id],
                |row| row.get(0),
            )?;
            if active {
                enqueue_summary_invalidation(
                    &transaction,
                    &branch_project_id,
                    "document",
                    document_id,
                    &command.new_commit_id,
                    "restore",
                    &command.occurred_at,
                )?;
            } else {
                transaction.execute(
                    "DELETE FROM summary_invalidation
                     WHERE project_id = ?1 AND scope_type = 'document' AND scope_id = ?2",
                    params![branch_project_id, document_id],
                )?;
            }
        }
        let changed_block_ids = changes
            .iter()
            .filter(|change| change.entity_type == "block")
            .map(|change| change.entity_id.as_str())
            .collect::<BTreeSet<_>>();
        for block_id in changed_block_ids {
            let active: bool = transaction.query_row(
                "SELECT EXISTS(
                   SELECT 1 FROM block AS b
                   JOIN document AS d ON d.id = b.document_id
                   WHERE d.project_id = ?1 AND b.id = ?2
                     AND b.deleted_at IS NULL AND d.deleted_at IS NULL
                 )",
                params![branch_project_id, block_id],
                |row| row.get(0),
            )?;
            if active {
                enqueue_summary_invalidation(
                    &transaction,
                    &branch_project_id,
                    "block",
                    block_id,
                    &command.new_commit_id,
                    "restore",
                    &command.occurred_at,
                )?;
            } else {
                transaction.execute(
                    "DELETE FROM summary_invalidation
                     WHERE project_id = ?1 AND scope_type = 'block' AND scope_id = ?2",
                    params![branch_project_id, block_id],
                )?;
            }
        }
        transaction.execute(
            "INSERT INTO commit_parent(commit_id, parent_id, position) VALUES (?1, ?2, 0)",
            params![command.new_commit_id, previous_head],
        )?;
        for (position, change) in changes.iter().enumerate() {
            transaction.execute(
                "INSERT INTO change_set(commit_id, position, entity_type, entity_id, operation, before_hash, after_hash, edit_journal_id)
                 VALUES (?1, ?2, ?3, ?4, ?5, ?6, ?7, ?8)",
                params![
                    command.new_commit_id,
                    position as i64,
                    change.entity_type,
                    change.entity_id,
                    change.operation,
                    change.before_hash,
                    change.after_hash,
                    change.edit_id
                ],
            )?;
        }
        let branch_updated = transaction.execute(
            "UPDATE branch SET head_commit_id = ?1, updated_at = ?2 WHERE id = ?3 AND head_commit_id = ?4",
            params![
                command.new_commit_id,
                command.occurred_at,
                command.branch_id,
                previous_head
            ],
        )?;
        if branch_updated != 1 {
            return Err(StoreError::InvariantViolation(
                "branch head changed while restoring a snapshot".into(),
            ));
        }
        let project_updated = transaction.execute(
            "UPDATE project SET head_commit_id = ?1, revision = revision + 1, updated_at = ?2 WHERE id = ?3",
            params![
                command.new_commit_id,
                command.occurred_at,
                branch_project_id
            ],
        )?;
        if project_updated != 1 {
            return Err(StoreError::InvariantViolation(
                "project head update changed an unexpected row count during restore".into(),
            ));
        }
        transaction.commit()?;
        Ok(RestoreReceipt {
            snapshot_id: command.snapshot_id.clone(),
            commit_id: command.new_commit_id.clone(),
            previous_head_commit_id: previous_head,
            restored_root_hash: snapshot.commit.root_hash,
            changed_blocks: changes
                .iter()
                .filter(|change| change.entity_type == "block")
                .count(),
        })
    }

    pub fn search_blocks(
        &self,
        project_id: &str,
        query: &str,
        limit: usize,
    ) -> StoreResult<Vec<BlockSearchHit>> {
        if query.trim().is_empty() || limit == 0 || limit > 1000 {
            return Err(StoreError::Validation(
                "search query must be non-empty and limit must be 1..=1000".into(),
            ));
        }
        let mut statement = self.connection.prepare(
            "SELECT f.block_id, f.document_id, f.plain_text, bm25(block_fts)
             FROM block_fts AS f
             JOIN document AS d ON d.id = f.document_id
             WHERE d.project_id = ?1 AND d.deleted_at IS NULL AND block_fts MATCH ?2
             ORDER BY bm25(block_fts), f.block_id
             LIMIT ?3",
        )?;
        let hits = statement
            .query_map(params![project_id, query, limit as i64], |row| {
                Ok(BlockSearchHit {
                    block_id: row.get(0)?,
                    document_id: row.get(1)?,
                    plain_text: row.get(2)?,
                    rank: row.get(3)?,
                })
            })?
            .collect::<Result<_, _>>()?;
        Ok(hits)
    }

    pub fn backup_to(&self, destination: impl AsRef<Path>) -> StoreResult<()> {
        let destination = destination.as_ref();
        if destination.exists() {
            return Err(StoreError::Validation(format!(
                "backup destination already exists: {}",
                destination.display()
            )));
        }
        self.connection.backup(MAIN_DB, destination, None)?;
        let backup = Connection::open(destination)?;
        let result: String = backup.query_row("PRAGMA quick_check", [], |row| row.get(0))?;
        if result != "ok" {
            return Err(StoreError::InvariantViolation(format!(
                "backup quick_check returned {result}"
            )));
        }
        Ok(())
    }

    pub fn verify_invariants(&self) -> StoreResult<()> {
        let quick_check: String = self
            .connection
            .query_row("PRAGMA quick_check", [], |row| row.get(0))?;
        if quick_check != "ok" {
            return Err(StoreError::InvariantViolation(format!(
                "quick_check returned {quick_check}"
            )));
        }
        let mut foreign_keys = self.connection.prepare("PRAGMA foreign_key_check")?;
        if foreign_keys.query([])?.next()?.is_some() {
            return Err(StoreError::InvariantViolation(
                "foreign_key_check found violations".into(),
            ));
        }

        let broken_branch_heads: i64 = self.connection.query_row(
            "SELECT COUNT(*) FROM branch b
             LEFT JOIN commit_node c ON c.id = b.head_commit_id
             WHERE c.id IS NULL OR c.project_id <> b.project_id",
            [],
            |row| row.get(0),
        )?;
        if broken_branch_heads != 0 {
            return Err(StoreError::InvariantViolation(format!(
                "{broken_branch_heads} branch heads are invalid"
            )));
        }

        let broken_project_heads: i64 = self.connection.query_row(
            "SELECT COUNT(*) FROM project p
             LEFT JOIN commit_node c ON c.id = p.head_commit_id
             WHERE p.head_commit_id IS NOT NULL AND (c.id IS NULL OR c.project_id <> p.id)",
            [],
            |row| row.get(0),
        )?;
        if broken_project_heads != 0 {
            return Err(StoreError::InvariantViolation(format!(
                "{broken_project_heads} project heads are invalid"
            )));
        }

        let revision_mismatches: i64 = self.connection.query_row(
            "SELECT COUNT(*) FROM block b
             LEFT JOIN (SELECT block_id, MAX(new_revision) AS latest FROM edit_journal GROUP BY block_id) e
               ON e.block_id = b.id
             WHERE b.revision <> COALESCE(e.latest, 0)",
            [],
            |row| row.get(0),
        )?;
        if revision_mismatches != 0 {
            return Err(StoreError::InvariantViolation(format!(
                "{revision_mismatches} block revisions do not match the edit journal"
            )));
        }

        let active_blocks: i64 = self.connection.query_row(
            "SELECT COUNT(*) FROM block WHERE deleted_at IS NULL",
            [],
            |row| row.get(0),
        )?;
        let indexed_blocks: i64 =
            self.connection
                .query_row("SELECT COUNT(*) FROM block_fts", [], |row| row.get(0))?;
        if active_blocks != indexed_blocks {
            return Err(StoreError::InvariantViolation(format!(
                "FTS index count {indexed_blocks} does not match active block count {active_blocks}"
            )));
        }
        let invalid_summary_invalidations: i64 = self.connection.query_row(
            "SELECT COUNT(*)
             FROM summary_invalidation AS si
             JOIN project AS p ON p.id = si.project_id
             JOIN commit_node AS c ON c.id = si.source_commit_id
             WHERE c.project_id <> si.project_id
                OR (si.scope_type = 'project' AND si.scope_id <> si.project_id)
                OR (si.scope_type = 'document' AND NOT EXISTS(
                  SELECT 1 FROM document AS d
                  WHERE d.project_id = si.project_id AND d.id = si.scope_id
                    AND d.deleted_at IS NULL
                ))
                OR (si.scope_type = 'block' AND NOT EXISTS(
                  SELECT 1 FROM block AS b
                  JOIN document AS d ON d.id = b.document_id
                  WHERE d.project_id = si.project_id AND b.id = si.scope_id
                    AND b.deleted_at IS NULL AND d.deleted_at IS NULL
                ))",
            [],
            |row| row.get(0),
        )?;
        if invalid_summary_invalidations != 0 {
            return Err(StoreError::InvariantViolation(format!(
                "{invalid_summary_invalidations} summary invalidations are stale or invalid"
            )));
        }
        self.verify_operation_invariants()?;
        Ok(())
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
            |row| {
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
            },
        )
        .optional()?
        .ok_or_else(|| StoreError::NotFound {
            entity: "summary_record",
            id: format!("{scope_type}:{scope_id}"),
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
