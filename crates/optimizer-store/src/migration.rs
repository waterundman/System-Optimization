use rusqlite::{Connection, TransactionBehavior};

use crate::error::{StoreError, StoreResult};

pub const CURRENT_SCHEMA_VERSION: i64 = 1;

const MIGRATION_1: &str = r#"
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
    transaction.commit()?;
    Ok(())
}
