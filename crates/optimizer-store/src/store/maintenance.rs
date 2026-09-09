use super::*;

impl OptimizerStore {
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

        let broken_candidate_branches: i64 = self.connection.query_row(
            "SELECT COUNT(*)
             FROM patch_candidate_branch AS cb
             JOIN patch_review_head AS h ON h.proposal_id = cb.proposal_id
             JOIN operation_run AS r ON r.id = h.run_id
             JOIN branch AS b ON b.id = cb.branch_id
             JOIN commit_node AS c ON c.id = cb.commit_id
             JOIN materialized_snapshot AS s ON s.id = cb.snapshot_id
             WHERE b.project_id <> r.project_id
                OR c.project_id <> r.project_id
                OR s.project_id <> r.project_id
                OR b.head_commit_id <> cb.commit_id
                OR s.commit_id <> cb.commit_id
                OR s.root_hash <> c.root_hash",
            [],
            |row| row.get(0),
        )?;
        if broken_candidate_branches != 0 {
            return Err(StoreError::InvariantViolation(format!(
                "{broken_candidate_branches} candidate branches are invalid"
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
