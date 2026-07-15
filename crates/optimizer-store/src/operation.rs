use rusqlite::{OptionalExtension, Transaction, TransactionBehavior, params};

use crate::error::{StoreError, StoreResult};
use crate::model::{
    AppendReviewEvent, ContextPacketRecord, ModelUsageRecord, NewOperationLifecycleEvent,
    OperationArtifactKind, OperationArtifactRecord, OperationFailureRecord,
    OperationLifecycleEventRecord, OperationRunRecord, OperationState, PersistOperationBundle,
    ReviewDecision, ReviewEventKind, ReviewEventRecord, ReviewSessionRecord, ReviewSessionStatus,
};
use crate::store::OptimizerStore;

const MAX_CONTEXT_PACKET_BYTES: usize = 32 * 1024 * 1024;
const MAX_ARTIFACT_BYTES: usize = 16 * 1024 * 1024;
const MAX_REVIEW_EVENT_BYTES: usize = 1024 * 1024;
const MAX_LIFECYCLE_EVENTS: usize = 64;

impl OptimizerStore {
    pub fn persist_operation_bundle(
        &mut self,
        bundle: &PersistOperationBundle,
    ) -> StoreResult<OperationRunRecord> {
        validate_bundle(bundle)?;
        let transaction = self
            .connection
            .transaction_with_behavior(TransactionBehavior::Immediate)?;
        assert_commit_belongs_to_project(
            &transaction,
            &bundle.run.base_commit_id,
            &bundle.run.project_id,
        )?;

        if let Some(packet) = &bundle.context_packet {
            transaction.execute(
                "INSERT INTO context_packet_record(
                   id, operation_intent_id, project_id, base_commit_id, packet_hash, payload_json, created_at
                 ) VALUES (?1, ?2, ?3, ?4, ?5, ?6, ?7)",
                params![
                    packet.id,
                    packet.operation_intent_id,
                    packet.project_id,
                    packet.base_commit_id,
                    packet.packet_hash,
                    packet.payload_json,
                    packet.created_at,
                ],
            )?;
        }

        let usage = bundle.run.usage.as_ref();
        let failure = bundle.run.failure.as_ref();
        transaction.execute(
            "INSERT INTO operation_run(
               id, operation_intent_id, project_id, base_commit_id, provider_id, model, state,
               context_packet_id, response_id, finish_reason,
               input_tokens, output_tokens, total_tokens, cached_input_tokens, reasoning_tokens,
               failure_code, failure_message, failure_retriable, started_at, updated_at
             ) VALUES (
               ?1, ?2, ?3, ?4, ?5, ?6, ?7, ?8, ?9, ?10,
               ?11, ?12, ?13, ?14, ?15, ?16, ?17, ?18, ?19, ?20
             )",
            params![
                bundle.run.id,
                bundle.run.operation_intent_id,
                bundle.run.project_id,
                bundle.run.base_commit_id,
                bundle.run.provider_id,
                bundle.run.model,
                bundle.run.state.as_str(),
                bundle
                    .context_packet
                    .as_ref()
                    .map(|packet| packet.id.as_str()),
                bundle.run.response_id,
                bundle.run.finish_reason,
                usage.map(|value| value.input_tokens),
                usage.map(|value| value.output_tokens),
                usage.map(|value| value.total_tokens),
                usage.and_then(|value| value.cached_input_tokens),
                usage.and_then(|value| value.reasoning_tokens),
                failure.map(|value| value.code.as_str()),
                failure.map(|value| value.message.as_str()),
                failure.map(|value| i64::from(value.retriable)),
                bundle.run.started_at,
                bundle.run.updated_at,
            ],
        )?;

        for (position, event) in bundle.lifecycle_events.iter().enumerate() {
            transaction.execute(
                "INSERT INTO operation_lifecycle_event(
                   run_id, sequence, from_state, to_state, occurred_at, reason
                 ) VALUES (?1, ?2, ?3, ?4, ?5, ?6)",
                params![
                    bundle.run.id,
                    position as i64 + 1,
                    event.from_state.as_str(),
                    event.to_state.as_str(),
                    event.occurred_at,
                    event.reason,
                ],
            )?;
        }

        if let Some(artifact) = &bundle.artifact {
            transaction.execute(
                "INSERT INTO operation_artifact(
                   id, run_id, kind, binding_hash, payload_json, created_at
                 ) VALUES (?1, ?2, ?3, ?4, ?5, ?6)",
                params![
                    artifact.id,
                    bundle.run.id,
                    artifact.kind.as_str(),
                    artifact.binding_hash,
                    artifact.payload_json,
                    artifact.created_at,
                ],
            )?;
            if artifact.kind == OperationArtifactKind::PatchProposal {
                transaction.execute(
                    "INSERT INTO patch_review_head(
                       proposal_id, run_id, revision, status, updated_at
                     ) VALUES (?1, ?2, 0, 'review', ?3)",
                    params![artifact.id, bundle.run.id, artifact.created_at],
                )?;
            }
        }
        transaction.commit()?;
        self.get_operation_run(&bundle.run.id)
    }

    pub fn get_operation_run(&self, run_id: &str) -> StoreResult<OperationRunRecord> {
        let raw = self
            .connection
            .query_row(
                "SELECT id, operation_intent_id, project_id, base_commit_id, provider_id, model,
                        state, context_packet_id, response_id, finish_reason,
                        input_tokens, output_tokens, total_tokens, cached_input_tokens, reasoning_tokens,
                        failure_code, failure_message, failure_retriable, started_at, updated_at
                 FROM operation_run WHERE id = ?1",
                [run_id],
                read_raw_operation_run,
            )
            .optional()?
            .ok_or_else(|| StoreError::NotFound {
                entity: "operation_run",
                id: run_id.to_owned(),
            })?;
        raw.try_into_record()
    }

    pub fn get_context_packet(&self, packet_id: &str) -> StoreResult<ContextPacketRecord> {
        self.connection
            .query_row(
                "SELECT id, operation_intent_id, project_id, base_commit_id, packet_hash,
                        payload_json, created_at
                 FROM context_packet_record WHERE id = ?1",
                [packet_id],
                |row| {
                    Ok(ContextPacketRecord {
                        id: row.get(0)?,
                        operation_intent_id: row.get(1)?,
                        project_id: row.get(2)?,
                        base_commit_id: row.get(3)?,
                        packet_hash: row.get(4)?,
                        payload_json: row.get(5)?,
                        created_at: row.get(6)?,
                    })
                },
            )
            .optional()?
            .ok_or_else(|| StoreError::NotFound {
                entity: "context_packet",
                id: packet_id.to_owned(),
            })
    }

    pub fn get_operation_artifact(&self, run_id: &str) -> StoreResult<OperationArtifactRecord> {
        let raw = self
            .connection
            .query_row(
                "SELECT id, run_id, kind, binding_hash, payload_json, created_at
                 FROM operation_artifact WHERE run_id = ?1",
                [run_id],
                |row| {
                    Ok((
                        row.get::<_, String>(0)?,
                        row.get::<_, String>(1)?,
                        row.get::<_, String>(2)?,
                        row.get::<_, String>(3)?,
                        row.get::<_, String>(4)?,
                        row.get::<_, String>(5)?,
                    ))
                },
            )
            .optional()?
            .ok_or_else(|| StoreError::NotFound {
                entity: "operation_artifact",
                id: run_id.to_owned(),
            })?;
        let kind = OperationArtifactKind::parse(&raw.2).ok_or_else(|| {
            StoreError::InvariantViolation(format!("unknown operation artifact kind {}", raw.2))
        })?;
        Ok(OperationArtifactRecord {
            id: raw.0,
            run_id: raw.1,
            kind,
            binding_hash: raw.3,
            payload_json: raw.4,
            created_at: raw.5,
        })
    }

    pub fn list_operation_lifecycle_events(
        &self,
        run_id: &str,
    ) -> StoreResult<Vec<OperationLifecycleEventRecord>> {
        let mut statement = self.connection.prepare(
            "SELECT run_id, sequence, from_state, to_state, occurred_at, reason
             FROM operation_lifecycle_event WHERE run_id = ?1 ORDER BY sequence",
        )?;
        let rows = statement
            .query_map([run_id], |row| {
                Ok((
                    row.get::<_, String>(0)?,
                    row.get::<_, i64>(1)?,
                    row.get::<_, String>(2)?,
                    row.get::<_, String>(3)?,
                    row.get::<_, String>(4)?,
                    row.get::<_, Option<String>>(5)?,
                ))
            })?
            .collect::<Result<Vec<_>, _>>()?;
        rows.into_iter()
            .map(|raw| {
                Ok(OperationLifecycleEventRecord {
                    run_id: raw.0,
                    sequence: raw.1,
                    from_state: parse_operation_state(&raw.2)?,
                    to_state: parse_operation_state(&raw.3)?,
                    occurred_at: raw.4,
                    reason: raw.5,
                })
            })
            .collect()
    }

    pub fn get_review_session(&self, proposal_id: &str) -> StoreResult<ReviewSessionRecord> {
        let raw = self
            .connection
            .query_row(
                "SELECT proposal_id, run_id, revision, status, updated_at
                 FROM patch_review_head WHERE proposal_id = ?1",
                [proposal_id],
                |row| {
                    Ok((
                        row.get::<_, String>(0)?,
                        row.get::<_, String>(1)?,
                        row.get::<_, i64>(2)?,
                        row.get::<_, String>(3)?,
                        row.get::<_, String>(4)?,
                    ))
                },
            )
            .optional()?
            .ok_or_else(|| StoreError::NotFound {
                entity: "patch_review",
                id: proposal_id.to_owned(),
            })?;
        Ok(ReviewSessionRecord {
            proposal_id: raw.0,
            run_id: raw.1,
            revision: raw.2,
            status: parse_review_status(&raw.3)?,
            updated_at: raw.4,
        })
    }

    pub fn append_review_event(
        &mut self,
        command: &AppendReviewEvent,
    ) -> StoreResult<ReviewSessionRecord> {
        validate_review_event(command)?;
        let transaction = self
            .connection
            .transaction_with_behavior(TransactionBehavior::Immediate)?;
        let (run_id, actual_revision, actual_status_value): (String, i64, String) = transaction
            .query_row(
                "SELECT run_id, revision, status FROM patch_review_head WHERE proposal_id = ?1",
                [&command.proposal_id],
                |row| Ok((row.get(0)?, row.get(1)?, row.get(2)?)),
            )
            .optional()?
            .ok_or_else(|| StoreError::NotFound {
                entity: "patch_review",
                id: command.proposal_id.clone(),
            })?;
        let actual_status = parse_review_status(&actual_status_value)?;
        if actual_revision != command.expected_revision || actual_status != command.expected_status
        {
            return Err(StoreError::StateConflict {
                entity: "patch_review",
                id: command.proposal_id.clone(),
                expected_revision: command.expected_revision,
                actual_revision,
                expected_state: command.expected_status.as_str().into(),
                actual_state: actual_status.as_str().into(),
            });
        }
        assert_review_transition(command)?;
        let new_revision = actual_revision + 1;
        let sequence: i64 = transaction.query_row(
            "SELECT COALESCE(MAX(sequence), 0) + 1 FROM patch_review_event WHERE proposal_id = ?1",
            [&command.proposal_id],
            |row| row.get(0),
        )?;
        transaction.execute(
            "INSERT INTO patch_review_event(
               id, proposal_id, sequence, base_revision, new_revision, kind,
               previous_status, next_status, hunk_id, decision, payload_json, occurred_at
             ) VALUES (?1, ?2, ?3, ?4, ?5, ?6, ?7, ?8, ?9, ?10, ?11, ?12)",
            params![
                command.id,
                command.proposal_id,
                sequence,
                actual_revision,
                new_revision,
                command.kind.as_str(),
                actual_status.as_str(),
                command.next_status.as_str(),
                command.hunk_id,
                command.decision.map(ReviewDecision::as_str),
                command.payload_json,
                command.occurred_at,
            ],
        )?;
        let updated = transaction.execute(
            "UPDATE patch_review_head
             SET revision = ?1, status = ?2, updated_at = ?3
             WHERE proposal_id = ?4 AND revision = ?5 AND status = ?6",
            params![
                new_revision,
                command.next_status.as_str(),
                command.occurred_at,
                command.proposal_id,
                actual_revision,
                actual_status.as_str(),
            ],
        )?;
        if updated != 1 {
            return Err(StoreError::InvariantViolation(
                "review head update changed an unexpected row count".into(),
            ));
        }
        update_operation_state_for_review(
            &transaction,
            &run_id,
            command.next_status,
            &command.occurred_at,
            command.kind.as_str(),
        )?;
        transaction.commit()?;
        self.get_review_session(&command.proposal_id)
    }

    pub fn list_review_events(&self, proposal_id: &str) -> StoreResult<Vec<ReviewEventRecord>> {
        let mut statement = self.connection.prepare(
            "SELECT id, proposal_id, sequence, base_revision, new_revision, kind,
                    previous_status, next_status, hunk_id, decision, payload_json, occurred_at
             FROM patch_review_event WHERE proposal_id = ?1 ORDER BY sequence",
        )?;
        let rows = statement
            .query_map([proposal_id], |row| {
                Ok((
                    row.get::<_, String>(0)?,
                    row.get::<_, String>(1)?,
                    row.get::<_, i64>(2)?,
                    row.get::<_, i64>(3)?,
                    row.get::<_, i64>(4)?,
                    row.get::<_, String>(5)?,
                    row.get::<_, String>(6)?,
                    row.get::<_, String>(7)?,
                    row.get::<_, Option<String>>(8)?,
                    row.get::<_, Option<String>>(9)?,
                    row.get::<_, Option<String>>(10)?,
                    row.get::<_, String>(11)?,
                ))
            })?
            .collect::<Result<Vec<_>, _>>()?;
        rows.into_iter()
            .map(|raw| {
                Ok(ReviewEventRecord {
                    id: raw.0,
                    proposal_id: raw.1,
                    sequence: raw.2,
                    base_revision: raw.3,
                    new_revision: raw.4,
                    kind: ReviewEventKind::parse(&raw.5).ok_or_else(|| {
                        StoreError::InvariantViolation(format!(
                            "unknown review event kind {}",
                            raw.5
                        ))
                    })?,
                    previous_status: parse_review_status(&raw.6)?,
                    next_status: parse_review_status(&raw.7)?,
                    hunk_id: raw.8,
                    decision: raw.9.as_deref().map(parse_review_decision).transpose()?,
                    payload_json: raw.10,
                    occurred_at: raw.11,
                })
            })
            .collect()
    }

    pub(crate) fn verify_operation_invariants(&self) -> StoreResult<()> {
        for (label, query) in [
            (
                "operation lifecycle sequences have gaps",
                "SELECT COUNT(*) FROM (
                   SELECT run_id FROM operation_lifecycle_event
                   GROUP BY run_id HAVING COUNT(*) <> MAX(sequence)
                 )",
            ),
            (
                "operation current state differs from latest lifecycle event",
                "SELECT COUNT(*) FROM operation_run r
                 WHERE r.state <> COALESCE((
                   SELECT e.to_state FROM operation_lifecycle_event e
                   WHERE e.run_id = r.id ORDER BY e.sequence DESC LIMIT 1
                 ), '')",
            ),
            (
                "review head revision differs from immutable review log",
                "SELECT COUNT(*) FROM patch_review_head h
                 WHERE h.revision <> COALESCE((
                   SELECT MAX(e.new_revision) FROM patch_review_event e
                   WHERE e.proposal_id = h.proposal_id
                 ), 0)",
            ),
            (
                "review head status differs from immutable review log",
                "SELECT COUNT(*) FROM patch_review_head h
                 WHERE h.status <> COALESCE((
                   SELECT e.next_status FROM patch_review_event e
                   WHERE e.proposal_id = h.proposal_id ORDER BY e.sequence DESC LIMIT 1
                 ), 'review')",
            ),
            (
                "patch proposal/review head cardinality is invalid",
                "SELECT COUNT(*) FROM operation_artifact a
                 LEFT JOIN patch_review_head h ON h.proposal_id = a.id
                 WHERE (a.kind = 'patch_proposal' AND h.proposal_id IS NULL)
                    OR (a.kind = 'findings' AND h.proposal_id IS NOT NULL)",
            ),
            (
                "review status and operation state disagree",
                "SELECT COUNT(*) FROM patch_review_head h
                 JOIN operation_run r ON r.id = h.run_id
                 WHERE r.state <> CASE h.status
                   WHEN 'applied' THEN 'accepted'
                   WHEN 'rejected' THEN 'rejected'
                   WHEN 'conflicted' THEN 'conflicted'
                   ELSE 'review'
                 END",
            ),
            (
                "context packet binding differs from operation run",
                "SELECT COUNT(*) FROM operation_run r
                 JOIN context_packet_record c ON c.id = r.context_packet_id
                 WHERE c.project_id <> r.project_id
                    OR c.base_commit_id <> r.base_commit_id
                    OR c.operation_intent_id <> r.operation_intent_id",
            ),
            (
                "operation base commit belongs to another project",
                "SELECT COUNT(*) FROM operation_run r
                 JOIN commit_node c ON c.id = r.base_commit_id
                 WHERE c.project_id <> r.project_id",
            ),
            (
                "context packet base commit belongs to another project",
                "SELECT COUNT(*) FROM context_packet_record p
                 JOIN commit_node c ON c.id = p.base_commit_id
                 WHERE c.project_id <> p.project_id",
            ),
            (
                "operation artifact cardinality is invalid for the run state",
                "SELECT COUNT(*) FROM operation_run r
                 LEFT JOIN operation_artifact a ON a.run_id = r.id
                 WHERE (r.state IN ('review', 'accepted', 'rejected', 'conflicted') AND a.id IS NULL)
                    OR (r.state IN ('failed', 'cancelled') AND a.id IS NOT NULL)",
            ),
            (
                "operation failure columns disagree with the run state",
                "SELECT COUNT(*) FROM operation_run r
                 WHERE (r.state IN ('failed', 'cancelled') AND r.failure_code IS NULL)
                    OR (r.state NOT IN ('failed', 'cancelled') AND r.failure_code IS NOT NULL)",
            ),
        ] {
            let count: i64 = self.connection.query_row(query, [], |row| row.get(0))?;
            if count != 0 {
                return Err(StoreError::InvariantViolation(format!("{label}: {count}")));
            }
        }
        Ok(())
    }
}

struct RawOperationRun {
    id: String,
    operation_intent_id: String,
    project_id: String,
    base_commit_id: String,
    provider_id: String,
    model: String,
    state: String,
    context_packet_id: Option<String>,
    response_id: Option<String>,
    finish_reason: Option<String>,
    input_tokens: Option<i64>,
    output_tokens: Option<i64>,
    total_tokens: Option<i64>,
    cached_input_tokens: Option<i64>,
    reasoning_tokens: Option<i64>,
    failure_code: Option<String>,
    failure_message: Option<String>,
    failure_retriable: Option<i64>,
    started_at: String,
    updated_at: String,
}

impl RawOperationRun {
    fn try_into_record(self) -> StoreResult<OperationRunRecord> {
        let usage = match (self.input_tokens, self.output_tokens, self.total_tokens) {
            (Some(input_tokens), Some(output_tokens), Some(total_tokens)) => {
                Some(ModelUsageRecord {
                    input_tokens,
                    output_tokens,
                    total_tokens,
                    cached_input_tokens: self.cached_input_tokens,
                    reasoning_tokens: self.reasoning_tokens,
                })
            }
            (None, None, None) => None,
            _ => {
                return Err(StoreError::InvariantViolation(
                    "operation usage columns are partially populated".into(),
                ));
            }
        };
        let failure = match (
            self.failure_code,
            self.failure_message,
            self.failure_retriable,
        ) {
            (Some(code), Some(message), Some(retriable)) => Some(OperationFailureRecord {
                code,
                message,
                retriable: retriable == 1,
            }),
            (None, None, None) => None,
            _ => {
                return Err(StoreError::InvariantViolation(
                    "operation failure columns are partially populated".into(),
                ));
            }
        };
        Ok(OperationRunRecord {
            id: self.id,
            operation_intent_id: self.operation_intent_id,
            project_id: self.project_id,
            base_commit_id: self.base_commit_id,
            provider_id: self.provider_id,
            model: self.model,
            state: parse_operation_state(&self.state)?,
            context_packet_id: self.context_packet_id,
            response_id: self.response_id,
            finish_reason: self.finish_reason,
            usage,
            failure,
            started_at: self.started_at,
            updated_at: self.updated_at,
        })
    }
}

fn read_raw_operation_run(row: &rusqlite::Row<'_>) -> rusqlite::Result<RawOperationRun> {
    Ok(RawOperationRun {
        id: row.get(0)?,
        operation_intent_id: row.get(1)?,
        project_id: row.get(2)?,
        base_commit_id: row.get(3)?,
        provider_id: row.get(4)?,
        model: row.get(5)?,
        state: row.get(6)?,
        context_packet_id: row.get(7)?,
        response_id: row.get(8)?,
        finish_reason: row.get(9)?,
        input_tokens: row.get(10)?,
        output_tokens: row.get(11)?,
        total_tokens: row.get(12)?,
        cached_input_tokens: row.get(13)?,
        reasoning_tokens: row.get(14)?,
        failure_code: row.get(15)?,
        failure_message: row.get(16)?,
        failure_retriable: row.get(17)?,
        started_at: row.get(18)?,
        updated_at: row.get(19)?,
    })
}

fn validate_bundle(bundle: &PersistOperationBundle) -> StoreResult<()> {
    let run = &bundle.run;
    for (field, value) in [
        ("run.id", run.id.as_str()),
        ("run.operation_intent_id", run.operation_intent_id.as_str()),
        ("run.project_id", run.project_id.as_str()),
        ("run.base_commit_id", run.base_commit_id.as_str()),
        ("run.provider_id", run.provider_id.as_str()),
        ("run.model", run.model.as_str()),
        ("run.started_at", run.started_at.as_str()),
        ("run.updated_at", run.updated_at.as_str()),
    ] {
        non_empty(field, value)?;
    }
    if !matches!(
        run.provider_id.as_str(),
        "deepseek" | "qwen" | "kimi" | "minimax"
    ) {
        return validation("run.provider_id is unsupported");
    }
    if !matches!(
        run.state,
        OperationState::Review | OperationState::Failed | OperationState::Cancelled
    ) {
        return validation("persisted execution state must be review, failed or cancelled");
    }
    validate_usage(run.usage.as_ref())?;
    match run.state {
        OperationState::Review => {
            if bundle.context_packet.is_none()
                || bundle.artifact.is_none()
                || run.response_id.as_deref().is_none_or(str::is_empty)
                || run.finish_reason.as_deref().is_none_or(str::is_empty)
                || run.failure.is_some()
            {
                return validation(
                    "review runs require context, artifact, response and finish reason without failure",
                );
            }
        }
        OperationState::Failed | OperationState::Cancelled => {
            if bundle.artifact.is_some() || run.failure.is_none() {
                return validation(
                    "failed/cancelled runs require a failure and cannot contain an artifact",
                );
            }
        }
        _ => unreachable!(),
    }
    if let Some(failure) = &run.failure {
        non_empty("failure.code", &failure.code)?;
        non_empty("failure.message", &failure.message)?;
        if failure.message.len() > 4_000 {
            return validation("failure.message exceeds 4,000 UTF-8 bytes");
        }
    }
    let packet = bundle.context_packet.as_ref();
    if let Some(packet) = packet {
        for (field, value) in [
            ("context.id", packet.id.as_str()),
            (
                "context.operation_intent_id",
                packet.operation_intent_id.as_str(),
            ),
            ("context.project_id", packet.project_id.as_str()),
            ("context.base_commit_id", packet.base_commit_id.as_str()),
            ("context.packet_hash", packet.packet_hash.as_str()),
            ("context.created_at", packet.created_at.as_str()),
        ] {
            non_empty(field, value)?;
        }
        if packet.operation_intent_id != run.operation_intent_id
            || packet.project_id != run.project_id
            || packet.base_commit_id != run.base_commit_id
        {
            return validation("context packet binding does not match operation run");
        }
        validate_json_object(
            "context.payload_json",
            &packet.payload_json,
            MAX_CONTEXT_PACKET_BYTES,
        )?;
    }
    if let Some(artifact) = &bundle.artifact {
        for (field, value) in [
            ("artifact.id", artifact.id.as_str()),
            ("artifact.binding_hash", artifact.binding_hash.as_str()),
            ("artifact.created_at", artifact.created_at.as_str()),
        ] {
            non_empty(field, value)?;
        }
        validate_json_object(
            "artifact.payload_json",
            &artifact.payload_json,
            MAX_ARTIFACT_BYTES,
        )?;
    }
    validate_lifecycle(&bundle.lifecycle_events, run.state)?;
    Ok(())
}

fn validate_lifecycle(
    events: &[NewOperationLifecycleEvent],
    final_state: OperationState,
) -> StoreResult<()> {
    if events.is_empty() || events.len() > MAX_LIFECYCLE_EVENTS {
        return validation("lifecycle_events must contain 1..=64 transitions");
    }
    if events[0].from_state != OperationState::Draft {
        return validation("lifecycle must start from draft");
    }
    for (index, event) in events.iter().enumerate() {
        non_empty("lifecycle.occurred_at", &event.occurred_at)?;
        if let Some(reason) = &event.reason {
            non_empty("lifecycle.reason", reason)?;
        }
        if !valid_operation_transition(event.from_state, event.to_state) {
            return validation("lifecycle contains an invalid state transition");
        }
        if index > 0 && events[index - 1].to_state != event.from_state {
            return validation("lifecycle transitions are not contiguous");
        }
    }
    if events.last().map(|event| event.to_state) != Some(final_state) {
        return validation("lifecycle final state does not match operation run");
    }
    Ok(())
}

fn validate_usage(usage: Option<&ModelUsageRecord>) -> StoreResult<()> {
    let Some(usage) = usage else { return Ok(()) };
    if usage.input_tokens < 0
        || usage.output_tokens < 0
        || usage.total_tokens < usage.input_tokens + usage.output_tokens
        || usage.cached_input_tokens.is_some_and(|value| value < 0)
        || usage.reasoning_tokens.is_some_and(|value| value < 0)
    {
        return validation("model usage token counts are invalid");
    }
    Ok(())
}

fn validate_review_event(command: &AppendReviewEvent) -> StoreResult<()> {
    for (field, value) in [
        ("review.id", command.id.as_str()),
        ("review.proposal_id", command.proposal_id.as_str()),
        ("review.occurred_at", command.occurred_at.as_str()),
    ] {
        non_empty(field, value)?;
    }
    if command.expected_revision < 0 {
        return validation("review.expected_revision must be non-negative");
    }
    if let Some(hunk_id) = &command.hunk_id {
        non_empty("review.hunk_id", hunk_id)?;
    }
    if let Some(payload) = &command.payload_json {
        validate_json("review.payload_json", payload, MAX_REVIEW_EVENT_BYTES)?;
    }
    Ok(())
}

fn assert_review_transition(command: &AppendReviewEvent) -> StoreResult<()> {
    let valid = match command.kind {
        ReviewEventKind::Decision => {
            command
                .hunk_id
                .as_deref()
                .is_some_and(|value| !value.trim().is_empty())
                && command.decision.is_some()
                && matches!(
                    command.expected_status,
                    ReviewSessionStatus::Review | ReviewSessionStatus::Ready
                )
                && matches!(
                    command.next_status,
                    ReviewSessionStatus::Review
                        | ReviewSessionStatus::Ready
                        | ReviewSessionStatus::Rejected
                )
                && (command.next_status != ReviewSessionStatus::Rejected
                    || command.decision == Some(ReviewDecision::Rejected))
        }
        ReviewEventKind::Apply => {
            command.expected_status == ReviewSessionStatus::Ready
                && command.next_status == ReviewSessionStatus::Applied
                && command.hunk_id.is_none()
                && command.decision.is_none()
        }
        ReviewEventKind::Conflict => {
            matches!(
                command.expected_status,
                ReviewSessionStatus::Review | ReviewSessionStatus::Ready
            ) && command.next_status == ReviewSessionStatus::Conflicted
                && command.hunk_id.is_none()
                && command.decision.is_none()
        }
        ReviewEventKind::Rebase => {
            command.expected_status == ReviewSessionStatus::Conflicted
                && command.next_status == ReviewSessionStatus::Review
                && command.hunk_id.is_none()
                && command.decision.is_none()
        }
        ReviewEventKind::Reject => {
            matches!(
                command.expected_status,
                ReviewSessionStatus::Review
                    | ReviewSessionStatus::Ready
                    | ReviewSessionStatus::Conflicted
            ) && command.next_status == ReviewSessionStatus::Rejected
                && command.hunk_id.is_none()
                && command.decision.is_none()
        }
    };
    if !valid {
        return validation("review event fields or state transition are invalid");
    }
    Ok(())
}

fn update_operation_state_for_review(
    transaction: &Transaction<'_>,
    run_id: &str,
    review_status: ReviewSessionStatus,
    occurred_at: &str,
    reason: &str,
) -> StoreResult<()> {
    let desired = match review_status {
        ReviewSessionStatus::Review | ReviewSessionStatus::Ready => OperationState::Review,
        ReviewSessionStatus::Applied => OperationState::Accepted,
        ReviewSessionStatus::Rejected => OperationState::Rejected,
        ReviewSessionStatus::Conflicted => OperationState::Conflicted,
    };
    let current_value: String = transaction.query_row(
        "SELECT state FROM operation_run WHERE id = ?1",
        [run_id],
        |row| row.get(0),
    )?;
    let current = parse_operation_state(&current_value)?;
    if current == desired {
        return Ok(());
    }
    if !valid_operation_transition(current, desired) {
        return Err(StoreError::InvariantViolation(format!(
            "review would move operation from {} to {}",
            current.as_str(),
            desired.as_str()
        )));
    }
    let sequence: i64 = transaction.query_row(
        "SELECT COALESCE(MAX(sequence), 0) + 1 FROM operation_lifecycle_event WHERE run_id = ?1",
        [run_id],
        |row| row.get(0),
    )?;
    transaction.execute(
        "INSERT INTO operation_lifecycle_event(
           run_id, sequence, from_state, to_state, occurred_at, reason
         ) VALUES (?1, ?2, ?3, ?4, ?5, ?6)",
        params![
            run_id,
            sequence,
            current.as_str(),
            desired.as_str(),
            occurred_at,
            reason
        ],
    )?;
    let updated = transaction.execute(
        "UPDATE operation_run SET state = ?1, updated_at = ?2 WHERE id = ?3 AND state = ?4",
        params![desired.as_str(), occurred_at, run_id, current.as_str()],
    )?;
    if updated != 1 {
        return Err(StoreError::InvariantViolation(
            "operation state update changed an unexpected row count".into(),
        ));
    }
    Ok(())
}

fn assert_commit_belongs_to_project(
    transaction: &Transaction<'_>,
    commit_id: &str,
    project_id: &str,
) -> StoreResult<()> {
    let exists: bool = transaction.query_row(
        "SELECT EXISTS(SELECT 1 FROM commit_node WHERE id = ?1 AND project_id = ?2)",
        params![commit_id, project_id],
        |row| row.get(0),
    )?;
    if !exists {
        return Err(StoreError::NotFound {
            entity: "project_commit",
            id: format!("{project_id}/{commit_id}"),
        });
    }
    Ok(())
}

fn valid_operation_transition(from: OperationState, to: OperationState) -> bool {
    use OperationState::*;
    matches!(
        (from, to),
        (Draft, Compiling)
            | (Draft, Cancelled)
            | (Compiling, Preflight | Failed | Cancelled)
            | (Preflight, Queued | Failed | Cancelled)
            | (Queued, Streaming | Failed | Cancelled)
            | (Streaming, Validating | Failed | Cancelled)
            | (Validating, Review | Failed)
            | (Review, Accepted | Rejected | Conflicted)
            | (Conflicted, Review | Rejected)
    )
}

fn parse_operation_state(value: &str) -> StoreResult<OperationState> {
    OperationState::parse(value)
        .ok_or_else(|| StoreError::InvariantViolation(format!("unknown operation state {value}")))
}

fn parse_review_status(value: &str) -> StoreResult<ReviewSessionStatus> {
    ReviewSessionStatus::parse(value)
        .ok_or_else(|| StoreError::InvariantViolation(format!("unknown review status {value}")))
}

fn parse_review_decision(value: &str) -> StoreResult<ReviewDecision> {
    ReviewDecision::parse(value)
        .ok_or_else(|| StoreError::InvariantViolation(format!("unknown review decision {value}")))
}

fn non_empty(field: &str, value: &str) -> StoreResult<()> {
    if value.trim().is_empty() {
        return validation(format!("{field} must not be empty"));
    }
    if value.len() > 4_000 {
        return validation(format!("{field} exceeds 4,000 UTF-8 bytes"));
    }
    Ok(())
}

fn validate_json_object(field: &str, value: &str, limit: usize) -> StoreResult<()> {
    validate_json(field, value, limit)?;
    if !serde_json::from_str::<serde_json::Value>(value).is_ok_and(|parsed| parsed.is_object()) {
        return validation(format!("{field} must be a JSON object"));
    }
    Ok(())
}

fn validate_json(field: &str, value: &str, limit: usize) -> StoreResult<()> {
    if value.len() > limit {
        return validation(format!("{field} exceeds {limit} UTF-8 bytes"));
    }
    serde_json::from_str::<serde_json::Value>(value)
        .map_err(|error| StoreError::Validation(format!("{field} is invalid JSON: {error}")))?;
    Ok(())
}

fn validation<T>(message: impl Into<String>) -> StoreResult<T> {
    Err(StoreError::Validation(message.into()))
}
