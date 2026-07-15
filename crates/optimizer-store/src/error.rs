use std::fmt;

#[derive(Debug)]
pub enum StoreError {
    Sqlite(rusqlite::Error),
    UnsafeSqliteVersion {
        actual: String,
        minimum: &'static str,
    },
    Validation(String),
    Snapshot(String),
    NotFound {
        entity: &'static str,
        id: String,
    },
    Conflict {
        entity: &'static str,
        id: String,
        expected_revision: i64,
        actual_revision: i64,
        expected_hash: String,
        actual_hash: String,
    },
    StateConflict {
        entity: &'static str,
        id: String,
        expected_revision: i64,
        actual_revision: i64,
        expected_state: String,
        actual_state: String,
    },
    InvariantViolation(String),
}

impl fmt::Display for StoreError {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            Self::Sqlite(error) => write!(formatter, "sqlite error: {error}"),
            Self::UnsafeSqliteVersion { actual, minimum } => {
                write!(
                    formatter,
                    "SQLite {actual} is unsafe for this store; minimum is {minimum}"
                )
            }
            Self::Validation(message) => write!(formatter, "validation error: {message}"),
            Self::Snapshot(message) => write!(formatter, "snapshot error: {message}"),
            Self::NotFound { entity, id } => write!(formatter, "{entity} not found: {id}"),
            Self::Conflict {
                entity,
                id,
                expected_revision,
                actual_revision,
                expected_hash,
                actual_hash,
            } => write!(
                formatter,
                "{entity} {id} changed: expected revision/hash {expected_revision}/{expected_hash}, actual {actual_revision}/{actual_hash}"
            ),
            Self::StateConflict {
                entity,
                id,
                expected_revision,
                actual_revision,
                expected_state,
                actual_state,
            } => write!(
                formatter,
                "{entity} {id} changed: expected revision/state {expected_revision}/{expected_state}, actual {actual_revision}/{actual_state}"
            ),
            Self::InvariantViolation(message) => {
                write!(formatter, "store invariant violated: {message}")
            }
        }
    }
}

impl std::error::Error for StoreError {}

impl From<rusqlite::Error> for StoreError {
    fn from(value: rusqlite::Error) -> Self {
        Self::Sqlite(value)
    }
}

pub type StoreResult<T> = Result<T, StoreError>;
