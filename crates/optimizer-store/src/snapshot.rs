use std::fmt;
use std::io::{Cursor, Read};

use serde::{Deserialize, Serialize};
use sha2::{Digest, Sha256};

pub const SNAPSHOT_CODEC: &str = "optimizer-json+zstd";
pub const SNAPSHOT_CODEC_VERSION: i64 = 1;
pub const SNAPSHOT_SCHEMA_VERSION: u32 = 1;
const MAX_COMPRESSED_BYTES: usize = 64 * 1024 * 1024;
const MAX_DECOMPRESSED_BYTES: u64 = 256 * 1024 * 1024;

#[derive(Debug)]
pub enum SnapshotCodecError {
    InvalidDescriptor(String),
    PayloadTooLarge { actual: usize, maximum: usize },
    ChecksumMismatch,
    Compression(std::io::Error),
    Json(serde_json::Error),
}

impl fmt::Display for SnapshotCodecError {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            Self::InvalidDescriptor(message) => write!(formatter, "invalid descriptor: {message}"),
            Self::PayloadTooLarge { actual, maximum } => {
                write!(
                    formatter,
                    "snapshot payload is {actual} bytes; maximum is {maximum}"
                )
            }
            Self::ChecksumMismatch => write!(formatter, "snapshot checksum does not match payload"),
            Self::Compression(error) => write!(formatter, "zstd error: {error}"),
            Self::Json(error) => write!(formatter, "JSON error: {error}"),
        }
    }
}

impl std::error::Error for SnapshotCodecError {}

impl From<serde_json::Error> for SnapshotCodecError {
    fn from(value: serde_json::Error) -> Self {
        Self::Json(value)
    }
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
pub struct ProjectSnapshotV1 {
    pub schema_version: u32,
    pub project: SnapshotProject,
    pub commit: SnapshotCommit,
    pub branches: Vec<SnapshotBranch>,
    pub documents: Vec<SnapshotDocument>,
    pub blocks: Vec<SnapshotBlock>,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
pub struct SnapshotProject {
    pub id: String,
    pub title: String,
    pub language: String,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
pub struct SnapshotCommit {
    pub id: String,
    pub root_hash: String,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
pub struct SnapshotBranch {
    pub id: String,
    pub name: String,
    pub head_commit_id: String,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
pub struct SnapshotDocument {
    pub id: String,
    pub parent_id: Option<String>,
    pub kind: String,
    pub title: String,
    pub order_key: String,
    pub revision: i64,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
pub struct SnapshotBlock {
    pub id: String,
    pub document_id: String,
    pub kind: String,
    pub order_key: String,
    pub content_json: String,
    pub plain_text: String,
    pub content_hash: String,
    pub revision: i64,
    pub locked: bool,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct EncodedSnapshot {
    pub payload: Vec<u8>,
    pub checksum: String,
}

pub fn encode_snapshot(
    snapshot: &ProjectSnapshotV1,
) -> Result<EncodedSnapshot, SnapshotCodecError> {
    validate_snapshot(snapshot)?;
    let json = serde_json::to_vec(snapshot)?;
    let payload =
        zstd::stream::encode_all(Cursor::new(json), 3).map_err(SnapshotCodecError::Compression)?;
    if payload.len() > MAX_COMPRESSED_BYTES {
        return Err(SnapshotCodecError::PayloadTooLarge {
            actual: payload.len(),
            maximum: MAX_COMPRESSED_BYTES,
        });
    }
    Ok(EncodedSnapshot {
        checksum: checksum(&payload),
        payload,
    })
}

pub fn decode_snapshot(
    payload: &[u8],
    expected_checksum: &str,
) -> Result<ProjectSnapshotV1, SnapshotCodecError> {
    if payload.len() > MAX_COMPRESSED_BYTES {
        return Err(SnapshotCodecError::PayloadTooLarge {
            actual: payload.len(),
            maximum: MAX_COMPRESSED_BYTES,
        });
    }
    if checksum(payload) != expected_checksum {
        return Err(SnapshotCodecError::ChecksumMismatch);
    }

    let decoder = zstd::stream::read::Decoder::new(Cursor::new(payload))
        .map_err(SnapshotCodecError::Compression)?;
    let mut limited = decoder.take(MAX_DECOMPRESSED_BYTES + 1);
    let mut json = Vec::new();
    limited
        .read_to_end(&mut json)
        .map_err(SnapshotCodecError::Compression)?;
    if json.len() as u64 > MAX_DECOMPRESSED_BYTES {
        return Err(SnapshotCodecError::PayloadTooLarge {
            actual: json.len(),
            maximum: MAX_DECOMPRESSED_BYTES as usize,
        });
    }
    let snapshot: ProjectSnapshotV1 = serde_json::from_slice(&json)?;
    validate_snapshot(&snapshot)?;
    Ok(snapshot)
}

pub fn checksum(payload: &[u8]) -> String {
    let digest = Sha256::digest(payload);
    format!("sha256:{}", hex::encode(digest))
}

fn validate_snapshot(snapshot: &ProjectSnapshotV1) -> Result<(), SnapshotCodecError> {
    if snapshot.schema_version != SNAPSHOT_SCHEMA_VERSION {
        return Err(SnapshotCodecError::InvalidDescriptor(format!(
            "schema version {} is unsupported",
            snapshot.schema_version
        )));
    }
    for (name, value) in [
        ("project.id", snapshot.project.id.as_str()),
        ("project.title", snapshot.project.title.as_str()),
        ("project.language", snapshot.project.language.as_str()),
        ("commit.id", snapshot.commit.id.as_str()),
        ("commit.rootHash", snapshot.commit.root_hash.as_str()),
    ] {
        if value.trim().is_empty() {
            return Err(SnapshotCodecError::InvalidDescriptor(format!(
                "{name} must not be empty"
            )));
        }
    }
    if snapshot
        .documents
        .windows(2)
        .any(|pair| pair[0].id >= pair[1].id)
    {
        return Err(SnapshotCodecError::InvalidDescriptor(
            "documents must be strictly ordered by id".into(),
        ));
    }
    if snapshot
        .blocks
        .windows(2)
        .any(|pair| pair[0].id >= pair[1].id)
    {
        return Err(SnapshotCodecError::InvalidDescriptor(
            "blocks must be strictly ordered by id".into(),
        ));
    }
    Ok(())
}
