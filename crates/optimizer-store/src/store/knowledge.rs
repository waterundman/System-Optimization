use super::*;

impl OptimizerStore {
    pub fn list_knowledge_items(&self, project_id: &str) -> StoreResult<Vec<KnowledgeItemRecord>> {
        if project_id.trim().is_empty() {
            return Err(StoreError::Validation(
                "project id must not be empty".into(),
            ));
        }
        let mut statement = self.connection.prepare(
            "SELECT id, project_id, kind, title, content, content_hash, status,
                    authority, sensitivity, severity, revision, created_at, updated_at
             FROM knowledge_item
             WHERE project_id = ?1
             ORDER BY status, kind, updated_at DESC, id",
        )?;
        statement
            .query_map([project_id], map_knowledge_item)?
            .collect::<Result<_, _>>()
            .map_err(StoreError::from)
    }

    pub fn create_knowledge_item(
        &mut self,
        command: &CreateKnowledgeItem,
    ) -> StoreResult<KnowledgeItemRecord> {
        validate_create_knowledge_item(command)?;
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
            "INSERT INTO knowledge_item(
               id, project_id, kind, title, content, content_hash, status,
               authority, sensitivity, severity, revision, created_at, updated_at
             ) VALUES (?1, ?2, ?3, ?4, ?5, ?6, 'canonical', ?7, ?8, ?9, 0, ?10, ?10)",
            params![
                command.id,
                command.project_id,
                command.kind,
                command.title,
                command.content,
                command.content_hash,
                command.authority,
                command.sensitivity,
                command.severity,
                command.created_at,
            ],
        )?;
        let result = read_knowledge_item(&transaction, &command.project_id, &command.id)?;
        transaction.commit()?;
        Ok(result)
    }

    pub fn set_knowledge_item_status(
        &mut self,
        command: &SetKnowledgeItemStatus,
    ) -> StoreResult<KnowledgeItemRecord> {
        validate_set_knowledge_item_status(command)?;
        let transaction = self
            .connection
            .transaction_with_behavior(TransactionBehavior::Immediate)?;
        let current = read_knowledge_item(&transaction, &command.project_id, &command.id)?;
        if current.revision != command.expected_revision {
            return Err(StoreError::StateConflict {
                entity: "knowledge_item",
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
            "UPDATE knowledge_item
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
                "knowledge item status update changed an unexpected row count".into(),
            ));
        }
        let result = read_knowledge_item(&transaction, &command.project_id, &command.id)?;
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
             ORDER BY CASE scope_type
                        WHEN 'block' THEN 0
                        WHEN 'document' THEN 1
                        WHEN 'project' THEN 2
                        ELSE 3
                      END,
                      created_at, scope_id
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

    pub fn get_ready_summary_record(
        &self,
        project_id: &str,
        scope_type: &str,
        scope_id: &str,
    ) -> StoreResult<Option<SummaryRecord>> {
        validate_summary_scope_reference(project_id, scope_type, scope_id)?;
        self.connection
            .query_row(
                "SELECT sr.project_id, sr.scope_type, sr.scope_id, sr.source_commit_id,
                        sr.source_hash, sr.summary_text, sr.summary_hash, sr.provider_id,
                        sr.model, sr.revision, sr.generated_at, sr.updated_at
                 FROM summary_record AS sr
                 WHERE sr.project_id = ?1 AND sr.scope_type = ?2 AND sr.scope_id = ?3
                   AND NOT EXISTS(
                     SELECT 1 FROM summary_invalidation AS si
                     WHERE si.project_id = sr.project_id
                       AND si.scope_type = sr.scope_type
                       AND si.scope_id = sr.scope_id
                   )",
                params![project_id, scope_type, scope_id],
                map_summary_record,
            )
            .optional()
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
}
