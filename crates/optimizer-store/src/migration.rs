use rusqlite::{Connection, TransactionBehavior};

use crate::error::{StoreError, StoreResult};

pub const CURRENT_SCHEMA_VERSION: i64 = 3;

pub(crate) const MIGRATION_1: &str = r#"
CREATE TABLE schema_migration (
  version INTEGER PRIMARY KEY,
  applied_at TEXT NOT NULL
) STRICT;

CREATE TABLE project (
  id TEXT PRIMARY KEY,
  title TEXT NOT NULL,
  language TEXT NOT NULL,
  schema_version INTEGER NOT NULL,
  head_commit_id TEXT,
  revision INTEGER NOT NULL DEFAULT 0 CHECK(revision >= 0),
  created_at TEXT NOT NULL,
  updated_at TEXT NOT NULL
) STRICT;

CREATE TABLE document (
  id TEXT PRIMARY KEY,
  project_id TEXT NOT NULL REFERENCES project(id),
  parent_id TEXT REFERENCES document(id),
  kind TEXT NOT NULL,
  title TEXT NOT NULL,
  order_key TEXT NOT NULL,
  revision INTEGER NOT NULL DEFAULT 0 CHECK(revision >= 0),
  deleted_at TEXT,
  UNIQUE(project_id, parent_id, order_key)
) STRICT;

CREATE TABLE block (
  id TEXT PRIMARY KEY,
  document_id TEXT NOT NULL REFERENCES document(id),
  kind TEXT NOT NULL,
  order_key TEXT NOT NULL,
  content_json TEXT NOT NULL CHECK(json_valid(content_json)),
  plain_text TEXT NOT NULL,
  content_hash TEXT NOT NULL,
  revision INTEGER NOT NULL DEFAULT 0 CHECK(revision >= 0),
  locked INTEGER NOT NULL DEFAULT 0 CHECK(locked IN (0, 1)),
  deleted_at TEXT,
  UNIQUE(document_id, order_key)
) STRICT;

CREATE TABLE commit_node (
  id TEXT PRIMARY KEY,
  project_id TEXT NOT NULL REFERENCES project(id),
  root_hash TEXT NOT NULL,
  reason TEXT NOT NULL,
  actor_type TEXT NOT NULL,
  actor_id TEXT,
  created_at TEXT NOT NULL
) STRICT;

CREATE TABLE commit_parent (
  commit_id TEXT NOT NULL REFERENCES commit_node(id),
  parent_id TEXT NOT NULL REFERENCES commit_node(id),
  position INTEGER NOT NULL CHECK(position >= 0),
  PRIMARY KEY(commit_id, parent_id),
  UNIQUE(commit_id, position),
  CHECK(commit_id <> parent_id)
) STRICT;

CREATE TABLE branch (
  id TEXT PRIMARY KEY,
  project_id TEXT NOT NULL REFERENCES project(id),
  name TEXT NOT NULL,
  head_commit_id TEXT NOT NULL REFERENCES commit_node(id),
  created_at TEXT NOT NULL,
  updated_at TEXT NOT NULL,
  UNIQUE(project_id, name)
) STRICT;

CREATE TABLE edit_journal (
  id TEXT PRIMARY KEY,
  project_id TEXT NOT NULL REFERENCES project(id),
  block_id TEXT NOT NULL REFERENCES block(id),
  base_revision INTEGER NOT NULL CHECK(base_revision >= 0),
  new_revision INTEGER NOT NULL CHECK(new_revision = base_revision + 1),
  before_hash TEXT NOT NULL,
  after_hash TEXT NOT NULL,
  before_content_json TEXT NOT NULL CHECK(json_valid(before_content_json)),
  after_content_json TEXT NOT NULL CHECK(json_valid(after_content_json)),
  before_plain_text TEXT NOT NULL,
  after_plain_text TEXT NOT NULL,
  occurred_at TEXT NOT NULL
) STRICT;

CREATE TABLE change_set (
  commit_id TEXT NOT NULL REFERENCES commit_node(id),
  position INTEGER NOT NULL CHECK(position >= 0),
  entity_type TEXT NOT NULL,
  entity_id TEXT NOT NULL,
  operation TEXT NOT NULL,
  before_hash TEXT,
  after_hash TEXT,
  edit_journal_id TEXT REFERENCES edit_journal(id),
  PRIMARY KEY(commit_id, position)
) STRICT;

CREATE TABLE materialized_snapshot (
  id TEXT PRIMARY KEY,
  project_id TEXT NOT NULL REFERENCES project(id),
  commit_id TEXT NOT NULL REFERENCES commit_node(id),
  root_hash TEXT NOT NULL,
  codec TEXT NOT NULL,
  codec_version INTEGER NOT NULL CHECK(codec_version >= 1),
  payload BLOB NOT NULL,
  checksum TEXT NOT NULL,
  created_at TEXT NOT NULL,
  UNIQUE(project_id, commit_id, codec, codec_version)
) STRICT;

CREATE INDEX document_project_parent_idx ON document(project_id, parent_id, order_key);
CREATE INDEX block_document_order_idx ON block(document_id, order_key);
CREATE INDEX commit_project_created_idx ON commit_node(project_id, created_at, id);
CREATE INDEX edit_project_block_idx ON edit_journal(project_id, block_id, new_revision);
CREATE INDEX snapshot_project_created_idx ON materialized_snapshot(project_id, created_at, id);

CREATE VIRTUAL TABLE block_fts USING fts5(
  block_id UNINDEXED,
  document_id UNINDEXED,
  plain_text,
  tokenize = 'unicode61'
);

CREATE TRIGGER block_fts_after_insert AFTER INSERT ON block WHEN new.deleted_at IS NULL BEGIN
  INSERT INTO block_fts(block_id, document_id, plain_text)
  VALUES (new.id, new.document_id, new.plain_text);
END;

CREATE TRIGGER block_fts_after_update AFTER UPDATE OF plain_text, deleted_at ON block BEGIN
  DELETE FROM block_fts WHERE block_id = old.id;
  INSERT INTO block_fts(block_id, document_id, plain_text)
  SELECT new.id, new.document_id, new.plain_text WHERE new.deleted_at IS NULL;
END;

CREATE TRIGGER block_fts_after_delete AFTER DELETE ON block BEGIN
  DELETE FROM block_fts WHERE block_id = old.id;
END;

CREATE TRIGGER commit_node_immutable_update BEFORE UPDATE ON commit_node BEGIN
  SELECT RAISE(ABORT, 'immutable:commit_node');
END;
CREATE TRIGGER commit_node_immutable_delete BEFORE DELETE ON commit_node BEGIN
  SELECT RAISE(ABORT, 'immutable:commit_node');
END;
CREATE TRIGGER commit_parent_immutable_update BEFORE UPDATE ON commit_parent BEGIN
  SELECT RAISE(ABORT, 'immutable:commit_parent');
END;
CREATE TRIGGER commit_parent_immutable_delete BEFORE DELETE ON commit_parent BEGIN
  SELECT RAISE(ABORT, 'immutable:commit_parent');
END;
CREATE TRIGGER edit_journal_immutable_update BEFORE UPDATE ON edit_journal BEGIN
  SELECT RAISE(ABORT, 'immutable:edit_journal');
END;
CREATE TRIGGER edit_journal_immutable_delete BEFORE DELETE ON edit_journal BEGIN
  SELECT RAISE(ABORT, 'immutable:edit_journal');
END;
CREATE TRIGGER change_set_immutable_update BEFORE UPDATE ON change_set BEGIN
  SELECT RAISE(ABORT, 'immutable:change_set');
END;
CREATE TRIGGER change_set_immutable_delete BEFORE DELETE ON change_set BEGIN
  SELECT RAISE(ABORT, 'immutable:change_set');
END;
CREATE TRIGGER snapshot_immutable_update BEFORE UPDATE ON materialized_snapshot BEGIN
  SELECT RAISE(ABORT, 'immutable:materialized_snapshot');
END;
CREATE TRIGGER snapshot_immutable_delete BEFORE DELETE ON materialized_snapshot BEGIN
  SELECT RAISE(ABORT, 'immutable:materialized_snapshot');
END;
"#;

const MIGRATION_2: &str = r#"
CREATE TABLE context_packet_record (
  id TEXT PRIMARY KEY,
  operation_intent_id TEXT NOT NULL,
  project_id TEXT NOT NULL REFERENCES project(id),
  base_commit_id TEXT NOT NULL REFERENCES commit_node(id),
  packet_hash TEXT NOT NULL,
  payload_json TEXT NOT NULL CHECK(json_valid(payload_json) AND json_type(payload_json) = 'object'),
  created_at TEXT NOT NULL
) STRICT;

CREATE TABLE operation_run (
  id TEXT PRIMARY KEY,
  operation_intent_id TEXT NOT NULL,
  project_id TEXT NOT NULL REFERENCES project(id),
  base_commit_id TEXT NOT NULL REFERENCES commit_node(id),
  provider_id TEXT NOT NULL CHECK(provider_id IN ('deepseek', 'qwen', 'kimi', 'minimax')),
  model TEXT NOT NULL,
  state TEXT NOT NULL CHECK(state IN (
    'draft', 'compiling', 'preflight', 'queued', 'streaming', 'validating',
    'review', 'accepted', 'rejected', 'conflicted', 'failed', 'cancelled'
  )),
  context_packet_id TEXT UNIQUE REFERENCES context_packet_record(id),
  response_id TEXT,
  finish_reason TEXT,
  input_tokens INTEGER CHECK(input_tokens IS NULL OR input_tokens >= 0),
  output_tokens INTEGER CHECK(output_tokens IS NULL OR output_tokens >= 0),
  total_tokens INTEGER CHECK(total_tokens IS NULL OR total_tokens >= 0),
  cached_input_tokens INTEGER CHECK(cached_input_tokens IS NULL OR cached_input_tokens >= 0),
  reasoning_tokens INTEGER CHECK(reasoning_tokens IS NULL OR reasoning_tokens >= 0),
  failure_code TEXT,
  failure_message TEXT,
  failure_retriable INTEGER CHECK(failure_retriable IS NULL OR failure_retriable IN (0, 1)),
  started_at TEXT NOT NULL,
  updated_at TEXT NOT NULL,
  CHECK(
    (input_tokens IS NULL AND output_tokens IS NULL AND total_tokens IS NULL)
    OR (input_tokens IS NOT NULL AND output_tokens IS NOT NULL AND total_tokens IS NOT NULL
        AND total_tokens >= input_tokens + output_tokens)
  ),
  CHECK(
    (failure_code IS NULL AND failure_message IS NULL AND failure_retriable IS NULL)
    OR (failure_code IS NOT NULL AND failure_message IS NOT NULL AND failure_retriable IS NOT NULL)
  )
) STRICT;

CREATE TABLE operation_lifecycle_event (
  run_id TEXT NOT NULL REFERENCES operation_run(id),
  sequence INTEGER NOT NULL CHECK(sequence >= 1),
  from_state TEXT NOT NULL CHECK(from_state IN (
    'draft', 'compiling', 'preflight', 'queued', 'streaming', 'validating',
    'review', 'accepted', 'rejected', 'conflicted', 'failed', 'cancelled'
  )),
  to_state TEXT NOT NULL CHECK(to_state IN (
    'draft', 'compiling', 'preflight', 'queued', 'streaming', 'validating',
    'review', 'accepted', 'rejected', 'conflicted', 'failed', 'cancelled'
  )),
  occurred_at TEXT NOT NULL,
  reason TEXT,
  PRIMARY KEY(run_id, sequence)
) STRICT;

CREATE TABLE operation_artifact (
  id TEXT PRIMARY KEY,
  run_id TEXT NOT NULL UNIQUE REFERENCES operation_run(id),
  kind TEXT NOT NULL CHECK(kind IN ('patch_proposal', 'findings')),
  binding_hash TEXT NOT NULL,
  payload_json TEXT NOT NULL CHECK(json_valid(payload_json) AND json_type(payload_json) = 'object'),
  created_at TEXT NOT NULL
) STRICT;

CREATE TABLE patch_review_head (
  proposal_id TEXT PRIMARY KEY REFERENCES operation_artifact(id),
  run_id TEXT NOT NULL UNIQUE REFERENCES operation_run(id),
  revision INTEGER NOT NULL DEFAULT 0 CHECK(revision >= 0),
  status TEXT NOT NULL CHECK(status IN ('review', 'ready', 'applied', 'rejected', 'conflicted')),
  updated_at TEXT NOT NULL
) STRICT;

CREATE TABLE patch_review_event (
  id TEXT PRIMARY KEY,
  proposal_id TEXT NOT NULL REFERENCES patch_review_head(proposal_id),
  sequence INTEGER NOT NULL CHECK(sequence >= 1),
  base_revision INTEGER NOT NULL CHECK(base_revision >= 0),
  new_revision INTEGER NOT NULL CHECK(new_revision = base_revision + 1),
  kind TEXT NOT NULL CHECK(kind IN ('decision', 'apply', 'conflict', 'rebase', 'reject')),
  previous_status TEXT NOT NULL CHECK(previous_status IN ('review', 'ready', 'applied', 'rejected', 'conflicted')),
  next_status TEXT NOT NULL CHECK(next_status IN ('review', 'ready', 'applied', 'rejected', 'conflicted')),
  hunk_id TEXT,
  decision TEXT CHECK(decision IS NULL OR decision IN ('accepted', 'rejected')),
  payload_json TEXT CHECK(payload_json IS NULL OR json_valid(payload_json)),
  occurred_at TEXT NOT NULL,
  UNIQUE(proposal_id, sequence),
  UNIQUE(proposal_id, new_revision)
) STRICT;

CREATE INDEX context_packet_project_created_idx
  ON context_packet_record(project_id, created_at, id);
CREATE INDEX operation_run_project_started_idx
  ON operation_run(project_id, started_at, id);
CREATE INDEX operation_event_run_idx
  ON operation_lifecycle_event(run_id, sequence);
CREATE INDEX review_event_proposal_idx
  ON patch_review_event(proposal_id, sequence);

CREATE TRIGGER context_packet_immutable_update BEFORE UPDATE ON context_packet_record BEGIN
  SELECT RAISE(ABORT, 'immutable:context_packet_record');
END;
CREATE TRIGGER context_packet_immutable_delete BEFORE DELETE ON context_packet_record BEGIN
  SELECT RAISE(ABORT, 'immutable:context_packet_record');
END;
CREATE TRIGGER operation_event_immutable_update BEFORE UPDATE ON operation_lifecycle_event BEGIN
  SELECT RAISE(ABORT, 'immutable:operation_lifecycle_event');
END;
CREATE TRIGGER operation_event_immutable_delete BEFORE DELETE ON operation_lifecycle_event BEGIN
  SELECT RAISE(ABORT, 'immutable:operation_lifecycle_event');
END;
CREATE TRIGGER operation_artifact_immutable_update BEFORE UPDATE ON operation_artifact BEGIN
  SELECT RAISE(ABORT, 'immutable:operation_artifact');
END;
CREATE TRIGGER operation_artifact_immutable_delete BEFORE DELETE ON operation_artifact BEGIN
  SELECT RAISE(ABORT, 'immutable:operation_artifact');
END;
CREATE TRIGGER review_event_immutable_update BEFORE UPDATE ON patch_review_event BEGIN
  SELECT RAISE(ABORT, 'immutable:patch_review_event');
END;
CREATE TRIGGER review_event_immutable_delete BEFORE DELETE ON patch_review_event BEGIN
  SELECT RAISE(ABORT, 'immutable:patch_review_event');
END;
CREATE TRIGGER review_head_no_delete BEFORE DELETE ON patch_review_head BEGIN
  SELECT RAISE(ABORT, 'immutable:patch_review_head');
END;
CREATE TRIGGER review_head_guard BEFORE UPDATE ON patch_review_head
WHEN new.proposal_id IS NOT old.proposal_id
  OR new.run_id IS NOT old.run_id
  OR new.revision <> old.revision + 1
  OR NOT (
    (old.status = 'review' AND new.status IN ('review', 'ready', 'rejected', 'conflicted'))
    OR (old.status = 'ready' AND new.status IN ('review', 'ready', 'applied', 'rejected', 'conflicted'))
    OR (old.status = 'conflicted' AND new.status IN ('review', 'rejected'))
  )
BEGIN
  SELECT RAISE(ABORT, 'invalid:patch_review_head_transition');
END;
CREATE TRIGGER operation_run_no_delete BEFORE DELETE ON operation_run BEGIN
  SELECT RAISE(ABORT, 'immutable:operation_run');
END;
CREATE TRIGGER operation_run_guard BEFORE UPDATE ON operation_run
WHEN new.id IS NOT old.id
  OR new.operation_intent_id IS NOT old.operation_intent_id
  OR new.project_id IS NOT old.project_id
  OR new.base_commit_id IS NOT old.base_commit_id
  OR new.provider_id IS NOT old.provider_id
  OR new.model IS NOT old.model
  OR new.context_packet_id IS NOT old.context_packet_id
  OR new.response_id IS NOT old.response_id
  OR new.finish_reason IS NOT old.finish_reason
  OR new.input_tokens IS NOT old.input_tokens
  OR new.output_tokens IS NOT old.output_tokens
  OR new.total_tokens IS NOT old.total_tokens
  OR new.cached_input_tokens IS NOT old.cached_input_tokens
  OR new.reasoning_tokens IS NOT old.reasoning_tokens
  OR new.failure_code IS NOT old.failure_code
  OR new.failure_message IS NOT old.failure_message
  OR new.failure_retriable IS NOT old.failure_retriable
  OR new.started_at IS NOT old.started_at
  OR NOT (
    (old.state = 'review' AND new.state IN ('accepted', 'rejected', 'conflicted'))
    OR (old.state = 'conflicted' AND new.state IN ('review', 'rejected'))
  )
BEGIN
  SELECT RAISE(ABORT, 'invalid:operation_run_transition');
END;
"#;

const MIGRATION_3: &str = r#"
CREATE TABLE style_sample (
  id TEXT PRIMARY KEY,
  project_id TEXT NOT NULL REFERENCES project(id),
  title TEXT NOT NULL,
  content TEXT NOT NULL,
  content_hash TEXT NOT NULL,
  status TEXT NOT NULL CHECK(status IN ('canonical', 'archived')),
  sensitivity TEXT NOT NULL CHECK(sensitivity IN ('local_sensitive', 'never_send')),
  revision INTEGER NOT NULL DEFAULT 0 CHECK(revision >= 0),
  created_at TEXT NOT NULL,
  updated_at TEXT NOT NULL
) STRICT;

CREATE INDEX style_sample_project_status_idx
  ON style_sample(project_id, status, updated_at, id);
"#;

pub fn migrate(connection: &mut Connection) -> StoreResult<()> {
    let current: i64 = connection.query_row("PRAGMA user_version", [], |row| row.get(0))?;
    if current > CURRENT_SCHEMA_VERSION {
        return Err(StoreError::Validation(format!(
            "database schema {current} is newer than supported {CURRENT_SCHEMA_VERSION}"
        )));
    }
    if current == CURRENT_SCHEMA_VERSION {
        return Ok(());
    }

    let transaction = connection.transaction_with_behavior(TransactionBehavior::Immediate)?;
    if current < 1 {
        transaction.execute_batch(MIGRATION_1)?;
        transaction.execute(
            "INSERT INTO schema_migration(version, applied_at) VALUES (1, strftime('%Y-%m-%dT%H:%M:%fZ', 'now'))",
            [],
        )?;
        transaction.pragma_update(None, "user_version", 1)?;
    }
    if current < 2 {
        transaction.execute_batch(MIGRATION_2)?;
        transaction.execute(
            "INSERT INTO schema_migration(version, applied_at) VALUES (2, strftime('%Y-%m-%dT%H:%M:%fZ', 'now'))",
            [],
        )?;
        transaction.pragma_update(None, "user_version", 2)?;
    }
    if current < 3 {
        transaction.execute_batch(MIGRATION_3)?;
        transaction.execute(
            "INSERT INTO schema_migration(version, applied_at) VALUES (3, strftime('%Y-%m-%dT%H:%M:%fZ', 'now'))",
            [],
        )?;
        transaction.pragma_update(None, "user_version", 3)?;
    }
    transaction.commit()?;
    Ok(())
}

#[cfg(test)]
mod tests {
    use rusqlite::Connection;

    use super::{CURRENT_SCHEMA_VERSION, MIGRATION_1, migrate};

    #[test]
    fn upgrades_a_version_one_database_and_remains_idempotent() {
        let mut connection = Connection::open_in_memory().unwrap();
        connection.execute_batch(MIGRATION_1).unwrap();
        connection
            .execute(
                "INSERT INTO schema_migration(version, applied_at) VALUES (1, '2026-07-14T00:00:00.000Z')",
                [],
            )
            .unwrap();
        connection.pragma_update(None, "user_version", 1).unwrap();

        migrate(&mut connection).unwrap();
        migrate(&mut connection).unwrap();
        let version: i64 = connection
            .query_row("PRAGMA user_version", [], |row| row.get(0))
            .unwrap();
        let operation_table: String = connection
            .query_row(
                "SELECT name FROM sqlite_schema WHERE type = 'table' AND name = 'operation_run'",
                [],
                |row| row.get(0),
            )
            .unwrap();
        let applied: i64 = connection
            .query_row("SELECT COUNT(*) FROM schema_migration", [], |row| {
                row.get(0)
            })
            .unwrap();
        assert_eq!(version, CURRENT_SCHEMA_VERSION);
        assert_eq!(operation_table, "operation_run");
        let style_table: String = connection
            .query_row(
                "SELECT name FROM sqlite_schema WHERE type = 'table' AND name = 'style_sample'",
                [],
                |row| row.get(0),
            )
            .unwrap();
        assert_eq!(style_table, "style_sample");
        assert_eq!(applied, 3);
    }
}
