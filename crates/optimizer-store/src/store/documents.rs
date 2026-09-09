use super::*;

impl OptimizerStore {
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
}
