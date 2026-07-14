use std::path::Path;
use std::time::Duration;

use rusqlite::{Connection, MAIN_DB, OptionalExtension, Transaction, TransactionBehavior, params};

use crate::error::{StoreError, StoreResult};
use crate::migration::{CURRENT_SCHEMA_VERSION, migrate};
use crate::model::{
    ApplyBlockEdit, BlockRecord, BlockSearchHit, CommitRecord, CreateSnapshot, EditReceipt,
    ProjectSeed, RestoreReceipt, RestoreSnapshot, SnapshotRecord,
};
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
}

pub struct OptimizerStore {
    connection: Connection,
}

impl OptimizerStore {
    pub fn open(path: impl AsRef<Path>) -> StoreResult<Self> {
        let mut connection = Connection::open(path)?;
        configure(&connection, true)?;
        ensure_safe_sqlite_version(&connection)?;
        migrate(&mut connection)?;
        let store = Self { connection };
        store.verify_invariants()?;
        Ok(store)
    }

    pub fn open_in_memory() -> StoreResult<Self> {
        let mut connection = Connection::open_in_memory()?;
        configure(&connection, false)?;
        ensure_safe_sqlite_version(&connection)?;
        migrate(&mut connection)?;
        Ok(Self { connection })
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

    pub fn get_block(&self, block_id: &str) -> StoreResult<BlockRecord> {
        self.connection
            .query_row(
                "SELECT id, document_id, kind, order_key, content_json, plain_text, content_hash, revision, locked
                 FROM block WHERE id = ?1 AND deleted_at IS NULL",
                [block_id],
                |row| {
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
                },
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
        if branch_project_id != current.project_id {
            return Err(StoreError::Validation(
                "branch and block belong to different projects".into(),
            ));
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

        let current_structure = read_structure_signature(&transaction, &branch_project_id)?;
        let snapshot_structure = snapshot_structure_signature(&snapshot);
        if current_structure != snapshot_structure {
            return Err(StoreError::Validation(
                "document or block structure changed; structural restore is not supported by codec v1"
                    .into(),
            ));
        }

        struct RestoredChange {
            edit_id: String,
            block_id: String,
            before_hash: String,
            after_hash: String,
        }
        let mut changes = Vec::new();
        for block in &snapshot.blocks {
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
                edit_id,
                block_id: block.id.clone(),
                before_hash: current.content_hash,
                after_hash: block.content_hash.clone(),
            });
        }
        if changes.is_empty() {
            return Err(StoreError::Validation(
                "snapshot already matches the current project state".into(),
            ));
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
        transaction.execute(
            "INSERT INTO commit_parent(commit_id, parent_id, position) VALUES (?1, ?2, 0)",
            params![command.new_commit_id, previous_head],
        )?;
        for (position, change) in changes.iter().enumerate() {
            transaction.execute(
                "INSERT INTO change_set(commit_id, position, entity_type, entity_id, operation, before_hash, after_hash, edit_journal_id)
                 VALUES (?1, ?2, 'block', ?3, 'restore', ?4, ?5, ?6)",
                params![
                    command.new_commit_id,
                    position as i64,
                    change.block_id,
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
            changed_blocks: changes.len(),
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
             WHERE d.project_id = ?1 AND block_fts MATCH ?2
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
        Ok(())
    }
}

struct EditableBlock {
    project_id: String,
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
            "SELECT d.project_id, b.revision, b.content_hash, b.content_json, b.plain_text, b.locked
             FROM block b JOIN document d ON d.id = b.document_id
             WHERE b.id = ?1 AND b.deleted_at IS NULL",
            [block_id],
            |row| {
                Ok(EditableBlock {
                    project_id: row.get(0)?,
                    revision: row.get(1)?,
                    content_hash: row.get(2)?,
                    content_json: row.get(3)?,
                    plain_text: row.get(4)?,
                    locked: row.get::<_, i64>(5)? == 1,
                })
            },
        )
        .optional()?
        .ok_or_else(|| StoreError::NotFound { entity: "block", id: block_id.to_owned() })
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
    Ok(())
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

#[derive(Debug, PartialEq, Eq)]
struct StructureSignature {
    documents: Vec<(String, Option<String>, String, String)>,
    blocks: Vec<(String, String, String, String, bool)>,
}

fn read_structure_signature(
    transaction: &Transaction<'_>,
    project_id: &str,
) -> StoreResult<StructureSignature> {
    let documents = read_active_documents(transaction, project_id)?
        .into_iter()
        .map(|document| {
            (
                document.id,
                document.parent_id,
                document.kind,
                document.order_key,
            )
        })
        .collect();
    let blocks = read_active_snapshot_blocks(transaction, project_id)?
        .into_iter()
        .map(|block| {
            (
                block.id,
                block.document_id,
                block.kind,
                block.order_key,
                block.locked,
            )
        })
        .collect();
    Ok(StructureSignature { documents, blocks })
}

fn snapshot_structure_signature(snapshot: &ProjectSnapshotV1) -> StructureSignature {
    StructureSignature {
        documents: snapshot
            .documents
            .iter()
            .map(|document| {
                (
                    document.id.clone(),
                    document.parent_id.clone(),
                    document.kind.clone(),
                    document.order_key.clone(),
                )
            })
            .collect(),
        blocks: snapshot
            .blocks
            .iter()
            .map(|block| {
                (
                    block.id.clone(),
                    block.document_id.clone(),
                    block.kind.clone(),
                    block.order_key.clone(),
                    block.locked,
                )
            })
            .collect(),
    }
}
