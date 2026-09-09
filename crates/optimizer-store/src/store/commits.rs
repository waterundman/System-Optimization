use super::*;

impl OptimizerStore {
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

    /// v0.7.0 Stage 1 (D1): return the ordered list of parent commit ids
    /// recorded in `commit_parent` for the given `commit_id`, ordered by
    /// `position` ascending so the first parent is the mainline and
    /// subsequent parents are branch merge heads. Returns an empty `Vec`
    /// for the seed commit (zero parents) and a single-element `Vec` for
    /// linear commits — both cases stay backward compatible with the
    /// v0.6.0 linear timeline renderer. The lookup reuses the existing
    /// `commit_parent` index and does not modify the schema; the caller
    /// (host layer) is expected to validate that `commit_id` belongs to
    /// `project_id` before invoking this method.
    pub fn list_commit_parents(
        &self,
        project_id: &str,
        commit_id: &str,
    ) -> StoreResult<Vec<String>> {
        let mut statement = self.connection.prepare(
            "SELECT cp.parent_id
             FROM commit_parent AS cp
             JOIN commit_node AS cn ON cn.id = cp.commit_id
             WHERE cp.commit_id = ?1 AND cn.project_id = ?2
             ORDER BY cp.position ASC",
        )?;
        let parents = statement
            .query_map(params![commit_id, project_id], |row| row.get(0))?
            .collect::<Result<_, _>>()?;
        Ok(parents)
    }

    /// Batch variant of [`list_commit_parents`] for timeline rendering, so the
    /// host can preload parent ids for all checkpoint commits with a bounded
    /// number of queries (chunks of at most 200 ids each). Returns a map from
    /// commit_id to its ordered parent ids; commits that do not belong to
    /// `project_id` are simply absent from the map.
    pub fn list_commit_parents_batch(
        &self,
        project_id: &str,
        commit_ids: &[String],
    ) -> StoreResult<BTreeMap<String, Vec<String>>> {
        let mut parents_by_commit = BTreeMap::new();
        for chunk in commit_ids.chunks(200) {
            let placeholders = (1..=chunk.len())
                .map(|index| format!("?{index}"))
                .collect::<Vec<_>>()
                .join(", ");
            let sql = format!(
                "SELECT cp.commit_id, cp.parent_id
                 FROM commit_parent AS cp
                 JOIN commit_node AS cn ON cn.id = cp.commit_id
                 WHERE cp.commit_id IN ({placeholders}) AND cn.project_id = ?{}
                 ORDER BY cp.commit_id ASC, cp.position ASC",
                chunk.len() + 1
            );
            let mut statement = self.connection.prepare(&sql)?;
            let rows = statement.query_map(
                rusqlite::params_from_iter(
                    chunk.iter().map(String::as_str).chain(std::iter::once(project_id)),
                ),
                |row| Ok((row.get::<_, String>(0)?, row.get::<_, String>(1)?)),
            )?;
            for row in rows {
                let (commit_id, parent_id) = row?;
                parents_by_commit
                    .entry(commit_id)
                    .or_insert_with(Vec::new)
                    .push(parent_id);
            }
        }
        Ok(parents_by_commit)
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
                 FROM materialized_snapshot AS s
                 WHERE project_id = ?1
                   AND NOT EXISTS(
                     SELECT 1 FROM patch_candidate_branch AS cb WHERE cb.snapshot_id = s.id
                   )
                 ORDER BY created_at DESC, id DESC LIMIT 1",
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
             FROM materialized_snapshot AS s
             WHERE project_id = ?1
               AND NOT EXISTS(
                 SELECT 1 FROM patch_candidate_branch AS cb WHERE cb.snapshot_id = s.id
               )
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

    pub fn head_snapshot(&self, project_id: &str) -> StoreResult<ProjectSnapshotV1> {
        let transaction = self.connection.unchecked_transaction()?;
        read_head_snapshot(&transaction, project_id)
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
}
