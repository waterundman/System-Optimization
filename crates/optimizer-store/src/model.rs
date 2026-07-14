#[derive(Debug, Clone, PartialEq, Eq)]
pub struct SeedDocument {
    pub id: String,
    pub parent_id: Option<String>,
    pub kind: String,
    pub title: String,
    pub order_key: String,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct SeedBlock {
    pub id: String,
    pub document_id: String,
    pub kind: String,
    pub order_key: String,
    pub content_json: String,
    pub plain_text: String,
    pub content_hash: String,
    pub locked: bool,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct ProjectSeed {
    pub project_id: String,
    pub title: String,
    pub language: String,
    pub initial_commit_id: String,
    pub initial_root_hash: String,
    pub main_branch_id: String,
    pub documents: Vec<SeedDocument>,
    pub blocks: Vec<SeedBlock>,
    pub created_at: String,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct BlockRecord {
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
pub struct ApplyBlockEdit {
    pub edit_id: String,
    pub commit_id: String,
    pub branch_id: String,
    pub block_id: String,
    pub expected_revision: i64,
    pub expected_hash: String,
    pub new_content_json: String,
    pub new_plain_text: String,
    pub new_content_hash: String,
    pub new_root_hash: String,
    pub reason: String,
    pub actor_type: String,
    pub actor_id: Option<String>,
    pub occurred_at: String,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct EditReceipt {
    pub edit_id: String,
    pub commit_id: String,
    pub previous_head_commit_id: String,
    pub block_id: String,
    pub new_revision: i64,
    pub new_content_hash: String,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct CommitRecord {
    pub id: String,
    pub project_id: String,
    pub root_hash: String,
    pub reason: String,
    pub actor_type: String,
    pub actor_id: Option<String>,
    pub created_at: String,
    pub parents: Vec<String>,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct CreateSnapshot {
    pub id: String,
    pub project_id: String,
    pub commit_id: String,
    pub root_hash: String,
    pub codec: String,
    pub codec_version: i64,
    pub payload: Vec<u8>,
    pub checksum: String,
    pub created_at: String,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct SnapshotRecord {
    pub id: String,
    pub project_id: String,
    pub commit_id: String,
    pub root_hash: String,
    pub codec: String,
    pub codec_version: i64,
    pub payload: Vec<u8>,
    pub checksum: String,
    pub created_at: String,
}

#[derive(Debug, Clone, PartialEq)]
pub struct BlockSearchHit {
    pub block_id: String,
    pub document_id: String,
    pub plain_text: String,
    pub rank: f64,
}
