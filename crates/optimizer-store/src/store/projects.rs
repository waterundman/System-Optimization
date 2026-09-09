use super::*;

impl OptimizerStore {
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

    pub fn create_review_candidate_branch(
        &mut self,
        command: &CreateReviewCandidateBranch,
    ) -> StoreResult<ReviewCandidateBranchRecord> {
        validate_review_candidate_branch(command)?;
        let transaction = self
            .connection
            .transaction_with_behavior(TransactionBehavior::Immediate)?;
        let binding = transaction
            .query_row(
                "SELECT r.project_id, r.base_commit_id, h.revision, h.status,
                        json_extract(a.payload_json, '$.target.documentId'),
                        json_extract(a.payload_json, '$.target.blockId'),
                        json_extract(a.payload_json, '$.target.baseRevision'),
                        json_extract(a.payload_json, '$.target.baseHash')
                 FROM patch_review_head AS h
                 JOIN operation_run AS r ON r.id = h.run_id
                 JOIN operation_artifact AS a ON a.id = h.proposal_id AND a.run_id = h.run_id
                 WHERE h.proposal_id = ?1 AND a.kind = 'patch_proposal'",
                [&command.proposal_id],
                |row| {
                    Ok((
                        row.get::<_, String>(0)?,
                        row.get::<_, String>(1)?,
                        row.get::<_, i64>(2)?,
                        row.get::<_, String>(3)?,
                        row.get::<_, String>(4)?,
                        row.get::<_, String>(5)?,
                        row.get::<_, i64>(6)?,
                        row.get::<_, String>(7)?,
                    ))
                },
            )
            .optional()?
            .ok_or_else(|| StoreError::NotFound {
                entity: "review_candidate",
                id: command.proposal_id.clone(),
            })?;
        let actual_status = ReviewSessionStatus::parse(&binding.3).ok_or_else(|| {
            StoreError::InvariantViolation(format!("unknown review status {}", binding.3))
        })?;
        if binding.2 != command.expected_review_revision
            || actual_status != ReviewSessionStatus::Ready
        {
            return Err(StoreError::StateConflict {
                entity: "patch_review",
                id: command.proposal_id.clone(),
                expected_revision: command.expected_review_revision,
                actual_revision: binding.2,
                expected_state: ReviewSessionStatus::Ready.as_str().into(),
                actual_state: actual_status.as_str().into(),
            });
        }
        if binding.0 != command.project_id
            || binding.1 != command.expected_project_head_commit_id
            || binding.4 != command.target_document_id
            || binding.5 != command.target_block_id
            || binding.6 != command.expected_block_revision
            || binding.7 != command.expected_block_hash
        {
            return Err(StoreError::Validation(
                "candidate branch command does not match its immutable proposal binding".into(),
            ));
        }
        let project_head: String = transaction
            .query_row(
                "SELECT head_commit_id FROM project WHERE id = ?1",
                [&command.project_id],
                |row| row.get(0),
            )
            .optional()?
            .ok_or_else(|| StoreError::NotFound {
                entity: "project",
                id: command.project_id.clone(),
            })?;
        if project_head != command.expected_project_head_commit_id {
            return Err(StoreError::StateConflict {
                entity: "candidate_branch_base",
                id: command.proposal_id.clone(),
                expected_revision: command.expected_review_revision,
                actual_revision: binding.2,
                expected_state: command.expected_project_head_commit_id.clone(),
                actual_state: project_head,
            });
        }
        let current = read_block_for_edit(&transaction, &command.target_block_id)?;
        if current.project_id != command.project_id
            || current.document_id != command.target_document_id
            || current.revision != command.expected_block_revision
            || current.content_hash != command.expected_block_hash
            || current.locked
        {
            return Err(StoreError::Conflict {
                entity: "candidate_branch_target",
                id: command.target_block_id.clone(),
                expected_revision: command.expected_block_revision,
                actual_revision: current.revision,
                expected_hash: command.expected_block_hash.clone(),
                actual_hash: current.content_hash,
            });
        }
        let already_exists: bool = transaction.query_row(
            "SELECT EXISTS(SELECT 1 FROM patch_candidate_branch WHERE proposal_id = ?1)",
            [&command.proposal_id],
            |row| row.get(0),
        )?;
        if already_exists {
            return Err(StoreError::Validation(
                "review candidate already has a branch".into(),
            ));
        }

        let mut snapshot = read_head_snapshot(&transaction, &command.project_id)?;
        if snapshot.commit.id != command.expected_project_head_commit_id {
            return Err(StoreError::InvariantViolation(
                "candidate snapshot does not match the expected project head".into(),
            ));
        }
        let target = snapshot
            .blocks
            .iter_mut()
            .find(|block| block.id == command.target_block_id)
            .ok_or_else(|| {
                StoreError::InvariantViolation(
                    "candidate target is absent from head snapshot".into(),
                )
            })?;
        target.content_json = command.new_content_json.clone();
        target.plain_text = command.new_plain_text.clone();
        target.content_hash = command.new_content_hash.clone();
        target.revision += 1;
        let target_document = snapshot
            .documents
            .iter_mut()
            .find(|document| document.id == command.target_document_id)
            .ok_or_else(|| {
                StoreError::InvariantViolation(
                    "candidate document is absent from head snapshot".into(),
                )
            })?;
        target_document.revision += 1;
        snapshot.commit.id = command.commit_id.clone();
        snapshot.commit.root_hash = command.new_root_hash.clone();
        snapshot.branches.push(SnapshotBranch {
            id: command.branch_id.clone(),
            name: command.branch_name.clone(),
            head_commit_id: command.commit_id.clone(),
        });
        snapshot
            .branches
            .sort_by(|left, right| left.id.cmp(&right.id));
        let encoded =
            encode_snapshot(&snapshot).map_err(|error| StoreError::Snapshot(error.to_string()))?;

        transaction.execute(
            "INSERT INTO commit_node(id, project_id, root_hash, reason, actor_type, actor_id, created_at)
             VALUES (?1, ?2, ?3, 'ai_candidate_branch', 'model', ?4, ?5)",
            params![
                command.commit_id,
                command.project_id,
                command.new_root_hash,
                command.proposal_id,
                command.occurred_at,
            ],
        )?;
        transaction.execute(
            "INSERT INTO commit_parent(commit_id, parent_id, position) VALUES (?1, ?2, 0)",
            params![command.commit_id, command.expected_project_head_commit_id],
        )?;
        transaction.execute(
            "INSERT INTO change_set(
               commit_id, position, entity_type, entity_id, operation,
               before_hash, after_hash, edit_journal_id
             ) VALUES (?1, 0, 'block', ?2, 'candidate', ?3, ?4, NULL)",
            params![
                command.commit_id,
                command.target_block_id,
                command.expected_block_hash,
                command.new_content_hash,
            ],
        )?;
        transaction.execute(
            "INSERT INTO branch(id, project_id, name, head_commit_id, created_at, updated_at)
             VALUES (?1, ?2, ?3, ?4, ?5, ?5)",
            params![
                command.branch_id,
                command.project_id,
                command.branch_name,
                command.commit_id,
                command.occurred_at,
            ],
        )?;
        transaction.execute(
            "INSERT INTO materialized_snapshot(
               id, project_id, commit_id, root_hash, codec, codec_version,
               payload, checksum, created_at
             ) VALUES (?1, ?2, ?3, ?4, ?5, ?6, ?7, ?8, ?9)",
            params![
                command.snapshot_id,
                command.project_id,
                command.commit_id,
                command.new_root_hash,
                SNAPSHOT_CODEC,
                SNAPSHOT_CODEC_VERSION,
                encoded.payload,
                encoded.checksum,
                command.occurred_at,
            ],
        )?;
        transaction.execute(
            "INSERT INTO patch_candidate_branch(
               proposal_id, branch_id, commit_id, snapshot_id, created_at
             ) VALUES (?1, ?2, ?3, ?4, ?5)",
            params![
                command.proposal_id,
                command.branch_id,
                command.commit_id,
                command.snapshot_id,
                command.occurred_at,
            ],
        )?;
        transaction.commit()?;
        Ok(ReviewCandidateBranchRecord {
            proposal_id: command.proposal_id.clone(),
            branch_id: command.branch_id.clone(),
            branch_name: command.branch_name.clone(),
            commit_id: command.commit_id.clone(),
            snapshot_id: command.snapshot_id.clone(),
            created_at: command.occurred_at.clone(),
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
}
