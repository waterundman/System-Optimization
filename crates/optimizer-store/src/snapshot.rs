use std::collections::HashMap;
use std::fmt;
use std::io::{Cursor, Read};

use serde::{Deserialize, Serialize};
use sha2::{Digest, Sha256};

use crate::model::{
    BlockDiffEntry, BlockDiffKind, DiffSummary, DocumentDiffResult, TextDiffOp,
};

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

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct ExportProjectSnapshot {
    pub schema_version: u32,
    pub project: SnapshotProject,
    pub commit: SnapshotCommit,
    pub branches: Vec<SnapshotBranch>,
    pub documents: Vec<SnapshotDocument>,
    pub blocks: Vec<ExportSnapshotBlock>,
}

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct ExportSnapshotBlock {
    pub id: String,
    pub document_id: String,
    pub kind: String,
    pub order_key: String,
    pub content_json: serde_json::Value,
    pub plain_text: String,
    pub content_hash: String,
    pub revision: i64,
    pub locked: bool,
}

impl ProjectSnapshotV1 {
    pub fn to_export(&self) -> ExportProjectSnapshot {
        let blocks = self
            .blocks
            .iter()
            .map(|block| {
                let content_json = serde_json::from_str(&block.content_json)
                    .unwrap_or(serde_json::Value::String(block.content_json.clone()));
                ExportSnapshotBlock {
                    id: block.id.clone(),
                    document_id: block.document_id.clone(),
                    kind: block.kind.clone(),
                    order_key: block.order_key.clone(),
                    content_json,
                    plain_text: block.plain_text.clone(),
                    content_hash: block.content_hash.clone(),
                    revision: block.revision,
                    locked: block.locked,
                }
            })
            .collect();
        ExportProjectSnapshot {
            schema_version: self.schema_version,
            project: self.project.clone(),
            commit: self.commit.clone(),
            branches: self.branches.clone(),
            documents: self.documents.clone(),
            blocks,
        }
    }
}

impl ExportProjectSnapshot {
    pub fn to_snapshot(&self) -> ProjectSnapshotV1 {
        let blocks = self
            .blocks
            .iter()
            .map(|block| {
                let content_json = if block.content_json.is_string() {
                    block
                        .content_json
                        .as_str()
                        .unwrap_or("")
                        .to_owned()
                } else {
                    serde_json::to_string(&block.content_json).unwrap_or_default()
                };
                SnapshotBlock {
                    id: block.id.clone(),
                    document_id: block.document_id.clone(),
                    kind: block.kind.clone(),
                    order_key: block.order_key.clone(),
                    content_json,
                    plain_text: block.plain_text.clone(),
                    content_hash: block.content_hash.clone(),
                    revision: block.revision,
                    locked: block.locked,
                }
            })
            .collect();
        ProjectSnapshotV1 {
            schema_version: self.schema_version,
            project: self.project.clone(),
            commit: self.commit.clone(),
            branches: self.branches.clone(),
            documents: self.documents.clone(),
            blocks,
        }
    }
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

/// Compare two snapshots' blocks for the given document ids and return a structured diff.
///
/// The diff uses a three-layer strategy:
/// 1. Block alignment via longest common subsequence keyed on `order_key`.
/// 2. `content_hash` comparison to distinguish unchanged from modified blocks.
/// 3. Myers text diff on `plain_text` for modified blocks.
///
/// # Stage 2 Optimizations
///
/// * **Early-stop heuristic**: for each LCS-aligned block pair, if `content_hash`
///   is identical the block is marked `Unchanged` and `myers_diff` is skipped
///   entirely (no text diff). This keeps the all-unchanged fast path O(N).
/// * **Change-region clipping**: `myers_diff` is invoked *only* for `Modified`
///   blocks. Pure `Added` / `Removed` blocks never trigger a text diff, so
///   large pure insertions / deletions stay cheap.
pub fn compare_documents(
    snapshot_a: &ProjectSnapshotV1,
    snapshot_b: &ProjectSnapshotV1,
    document_id_a: &str,
    document_id_b: &str,
) -> DocumentDiffResult {
    let mut blocks_a: Vec<&SnapshotBlock> = snapshot_a
        .blocks
        .iter()
        .filter(|b| b.document_id == document_id_a)
        .collect();
    blocks_a.sort_by(|a, b| a.order_key.cmp(&b.order_key));

    let mut blocks_b: Vec<&SnapshotBlock> = snapshot_b
        .blocks
        .iter()
        .filter(|b| b.document_id == document_id_b)
        .collect();
    blocks_b.sort_by(|a, b| a.order_key.cmp(&b.order_key));

    let matches = lcs_by_order_key(&blocks_a, &blocks_b);

    let mut entries: Vec<BlockDiffEntry> = Vec::new();
    let mut i = 0usize;
    let mut j = 0usize;
    let mut match_idx = 0usize;

    while i < blocks_a.len() || j < blocks_b.len() {
        if match_idx < matches.len() {
            let (next_i, next_j) = matches[match_idx];
            // Change-region clipping: pure removals before the next match
            // never call myers_diff.
            while i < next_i {
                entries.push(BlockDiffEntry {
                    block_id_a: Some(blocks_a[i].id.clone()),
                    block_id_b: None,
                    kind: BlockDiffKind::Removed,
                    text_diff: None,
                });
                i += 1;
            }
            // Change-region clipping: pure additions before the next match
            // never call myers_diff.
            while j < next_j {
                entries.push(BlockDiffEntry {
                    block_id_a: None,
                    block_id_b: Some(blocks_b[j].id.clone()),
                    kind: BlockDiffKind::Added,
                    text_diff: None,
                });
                j += 1;
            }
            let block_a = blocks_a[i];
            let block_b = blocks_b[j];
            // Early-stop heuristic: identical content_hash → Unchanged, skip
            // myers_diff. Only differing hashes fall through to text diff.
            if block_a.content_hash == block_b.content_hash {
                entries.push(BlockDiffEntry {
                    block_id_a: Some(block_a.id.clone()),
                    block_id_b: Some(block_b.id.clone()),
                    kind: BlockDiffKind::Unchanged,
                    text_diff: None,
                });
            } else {
                entries.push(BlockDiffEntry {
                    block_id_a: Some(block_a.id.clone()),
                    block_id_b: Some(block_b.id.clone()),
                    kind: BlockDiffKind::Modified,
                    text_diff: Some(diff_block_texts(&block_a.plain_text, &block_b.plain_text)),
                });
            }
            i += 1;
            j += 1;
            match_idx += 1;
        } else {
            // Trailing removals — no myers_diff.
            while i < blocks_a.len() {
                entries.push(BlockDiffEntry {
                    block_id_a: Some(blocks_a[i].id.clone()),
                    block_id_b: None,
                    kind: BlockDiffKind::Removed,
                    text_diff: None,
                });
                i += 1;
            }
            // Trailing additions — no myers_diff.
            while j < blocks_b.len() {
                entries.push(BlockDiffEntry {
                    block_id_a: None,
                    block_id_b: Some(blocks_b[j].id.clone()),
                    kind: BlockDiffKind::Added,
                    text_diff: None,
                });
                j += 1;
            }
        }
    }

    let summary = DiffSummary {
        added_count: entries
            .iter()
            .filter(|e| e.kind == BlockDiffKind::Added)
            .count() as i64,
        removed_count: entries
            .iter()
            .filter(|e| e.kind == BlockDiffKind::Removed)
            .count() as i64,
        modified_count: entries
            .iter()
            .filter(|e| e.kind == BlockDiffKind::Modified)
            .count() as i64,
        unchanged_count: entries
            .iter()
            .filter(|e| e.kind == BlockDiffKind::Unchanged)
            .count() as i64,
    };

    DocumentDiffResult {
        document_id_a: document_id_a.to_string(),
        document_id_b: document_id_b.to_string(),
        blocks: entries,
        summary,
    }
}

/// Compute the longest common subsequence of blocks keyed by `order_key`.
///
/// Returns a list of `(i, j)` index pairs into `a` and `b` forming a
/// non-crossing alignment where `a[i].order_key == b[j].order_key`.
///
/// # Stage 2 Optimization
///
/// The previous implementation was a classic O(N*M) DP which becomes
/// prohibitive for 10k × 10k inputs (100M cells). This version uses a
/// **hash-index acceleration path** with a DP fallback:
///
/// 1. Build `HashMap<order_key, Vec<index>>` from `b` — expected O(M).
/// 2. Generate match pairs `(i, j)` by looking up each `a[i]`'s `order_key`
///    in the map — expected O(N + P) where P is the number of pairs.
/// 3. Sort pairs by `(i asc, j desc)` and find the longest strictly-increasing
///    subsequence on `j` (patience sorting, O(P log P)). The `j desc` tiebreak
///    guarantees each `a`-index contributes at most one pair to the LIS,
///    correctly handling duplicate `order_key`s.
///
/// # Fallback
///
/// When match pairs are dense — i.e. `P > N*M/4` *and* the input is
/// non-trivial (`N*M > 4096`) — the LIS approach is no cheaper than DP, so we
/// fall back to the classic O(N*M) DP in [`lcs_by_order_key_dp`]. This keeps
/// small inputs on the hash path (for consistency with the unit tests) while
/// protecting large inputs with heavy duplication from pathological LIS runs.
pub fn lcs_by_order_key<'a>(
    a: &[&'a SnapshotBlock],
    b: &[&'a SnapshotBlock],
) -> Vec<(usize, usize)> {
    let (n, m) = (a.len(), b.len());
    if n == 0 || m == 0 {
        return Vec::new();
    }

    // 1. Build hash index: b's order_key → list of indices in b.
    let mut b_index: HashMap<&str, Vec<usize>> = HashMap::with_capacity(m);
    for (j, block) in b.iter().enumerate() {
        b_index.entry(block.order_key.as_str()).or_default().push(j);
    }

    // 2. Generate match pairs (i, j) where a[i].order_key == b[j].order_key.
    let mut pairs: Vec<(usize, usize)> = Vec::new();
    for (i, block) in a.iter().enumerate() {
        if let Some(indices) = b_index.get(block.order_key.as_str()) {
            for &j in indices {
                pairs.push((i, j));
            }
        }
    }

    if pairs.is_empty() {
        return Vec::new();
    }

    // Fallback: when match pairs are dense (heavy duplication) AND the input
    // is large enough that DP is competitive, defer to the classic DP. Small
    // inputs always stay on the hash path.
    let nm = n.saturating_mul(m);
    if nm > 4096 && pairs.len() > nm / 4 {
        return lcs_by_order_key_dp(a, b);
    }

    // 3. Sort by (i ascending, j descending). The j-descending tiebreak is
    //    essential: for the same i, only one pair can survive a strictly-
    //    increasing LIS on j, which enforces "each a-index used at most once".
    pairs.sort_unstable_by(|x, y| x.0.cmp(&y.0).then_with(|| y.1.cmp(&x.1)));

    // Patience-sorting LIS on j (strictly increasing).
    // tails[k] = smallest possible tail value of an increasing subseq of len k+1.
    // tails_idx[k] = index into `pairs` of the element currently holding tails[k].
    let mut tails: Vec<usize> = Vec::with_capacity(pairs.len());
    let mut tails_idx: Vec<usize> = Vec::with_capacity(pairs.len());
    // prev[idx] = index into `pairs` of the predecessor in the LIS ending at
    // pairs[idx], or -1 if none.
    let mut prev: Vec<i64> = vec![-1; pairs.len()];

    for (idx, &(_, j)) in pairs.iter().enumerate() {
        // bisect_left: first position where tails[pos] >= j.
        let pos = tails.partition_point(|&t| t < j);
        if pos == tails.len() {
            tails.push(j);
            tails_idx.push(idx);
        } else {
            tails[pos] = j;
            tails_idx[pos] = idx;
        }
        prev[idx] = if pos > 0 {
            tails_idx[pos - 1] as i64
        } else {
            -1
        };
    }

    // Reconstruct the LIS by following predecessor pointers from the last tail.
    let mut result: Vec<(usize, usize)> = Vec::with_capacity(tails.len());
    let mut cur = if tails.is_empty() {
        -1
    } else {
        tails_idx[tails.len() - 1] as i64
    };
    while cur >= 0 {
        result.push(pairs[cur as usize]);
        cur = prev[cur as usize];
    }
    result.reverse();
    result
}

/// Classic O(N*M) DP fallback for [`lcs_by_order_key`].
///
/// Invoked when the hash-index path does not pay off (dense duplicate
/// `order_key`s on large inputs). Kept as a separate function so the hot path
/// stays allocation-light.
fn lcs_by_order_key_dp<'a>(
    a: &[&'a SnapshotBlock],
    b: &[&'a SnapshotBlock],
) -> Vec<(usize, usize)> {
    let (n, m) = (a.len(), b.len());
    let mut dp = vec![vec![0usize; m + 1]; n + 1];
    for i in 1..=n {
        for j in 1..=m {
            if a[i - 1].order_key == b[j - 1].order_key {
                dp[i][j] = dp[i - 1][j - 1] + 1;
            } else {
                dp[i][j] = dp[i - 1][j].max(dp[i][j - 1]);
            }
        }
    }
    let mut matches: Vec<(usize, usize)> = Vec::new();
    let mut i = n;
    let mut j = m;
    while i > 0 && j > 0 {
        if a[i - 1].order_key == b[j - 1].order_key {
            matches.push((i - 1, j - 1));
            i -= 1;
            j -= 1;
        } else if dp[i - 1][j] >= dp[i][j - 1] {
            i -= 1;
        } else {
            j -= 1;
        }
    }
    matches.reverse();
    matches
}

/// Myers O(ND) diff algorithm producing a coalesced sequence of text operations.
/// Returns an error only if the trace fails to find a path (an internal
/// invariant; for well-formed inputs the algorithm always terminates with one).
fn myers_diff(a: &str, b: &str) -> Result<Vec<TextDiffOp>, SnapshotCodecError> {
    let a: Vec<char> = a.chars().collect();
    let b: Vec<char> = b.chars().collect();
    let (n, m) = (a.len(), b.len());

    if n == 0 && m == 0 {
        return Ok(Vec::new());
    }
    if n == 0 {
        return Ok(vec![TextDiffOp::Insert(b.iter().collect())]);
    }
    if m == 0 {
        return Ok(vec![TextDiffOp::Delete(a.iter().collect())]);
    }

    let max = n + m;
    let offset = max as i64;
    let mut v: Vec<i64> = vec![0; 2 * max + 1];
    let mut trace: Vec<Vec<i64>> = Vec::with_capacity(max + 1);
    let mut found_d: Option<usize> = None;

    for d in 0..=max as i64 {
        trace.push(v.clone());
        let mut reached = false;
        for k in (-d..=d).step_by(2) {
            let mut x = if k == -d {
                v[(k + 1 + offset) as usize]
            } else if k == d {
                v[(k - 1 + offset) as usize] + 1
            } else if v[(k - 1 + offset) as usize] < v[(k + 1 + offset) as usize] {
                v[(k + 1 + offset) as usize]
            } else {
                v[(k - 1 + offset) as usize] + 1
            };
            let mut y = x - k;
            while (x as usize) < n && (y as usize) < m && a[x as usize] == b[y as usize] {
                x += 1;
                y += 1;
            }
            v[(k + offset) as usize] = x;
            if (x as usize) >= n && (y as usize) >= m {
                found_d = Some(d as usize);
                reached = true;
                break;
            }
        }
        if reached {
            break;
        }
    }

    let d_final = found_d.ok_or_else(|| {
        SnapshotCodecError::InvalidDescriptor("Myers diff failed to find a path".into())
    })?;

    let mut ops: Vec<TextDiffOp> = Vec::new();
    let mut x = n as i64;
    let mut y = m as i64;

    for d in (1..=d_final).rev() {
        let prev_v = &trace[d];
        let k = x - y;
        let prev_k = if k == -(d as i64) {
            k + 1
        } else if k == d as i64 {
            k - 1
        } else if prev_v[(k - 1 + offset) as usize] < prev_v[(k + 1 + offset) as usize] {
            k + 1
        } else {
            k - 1
        };
        let prev_x = prev_v[(prev_k + offset) as usize];
        let prev_y = prev_x - prev_k;

        while x > prev_x && y > prev_y {
            ops.push(TextDiffOp::Equal(a[(x - 1) as usize].to_string()));
            x -= 1;
            y -= 1;
        }
        if x > prev_x {
            ops.push(TextDiffOp::Delete(a[(x - 1) as usize].to_string()));
            x -= 1;
        } else if y > prev_y {
            ops.push(TextDiffOp::Insert(b[(y - 1) as usize].to_string()));
            y -= 1;
        }
    }

    while x > 0 && y > 0 {
        ops.push(TextDiffOp::Equal(a[(x - 1) as usize].to_string()));
        x -= 1;
        y -= 1;
    }

    ops.reverse();
    Ok(coalesce_text_ops(ops))
}

/// Soft cap for the per-block Myers diff, in characters. Above this the trace
/// (which keeps one full `v` vector per D step) would consume O((n+m)^2)
/// memory, so we fall back to a coarse whole-block replace. Block-level
/// semantics (`Modified` / `Unchanged`) are unaffected.
const MYERS_DIFF_MAX_CHARS: usize = 1_048_576;

/// Coarse whole-block replace used for oversized blocks or as a defensive
/// fallback if `myers_diff` reports an internal trace failure.
fn replace_block_texts(a: &str, b: &str) -> Vec<TextDiffOp> {
    let mut ops = Vec::with_capacity(2);
    if !a.is_empty() {
        ops.push(TextDiffOp::Delete(a.to_string()));
    }
    if !b.is_empty() {
        ops.push(TextDiffOp::Insert(b.to_string()));
    }
    ops
}

/// Text diff for a modified block pair, honoring the Myers soft cap.
fn diff_block_texts(a: &str, b: &str) -> Vec<TextDiffOp> {
    let oversized = a.chars().count() > MYERS_DIFF_MAX_CHARS
        || b.chars().count() > MYERS_DIFF_MAX_CHARS;
    if oversized {
        return replace_block_texts(a, b);
    }
    myers_diff(a, b).unwrap_or_else(|_| replace_block_texts(a, b))
}

fn coalesce_text_ops(ops: Vec<TextDiffOp>) -> Vec<TextDiffOp> {
    let mut result: Vec<TextDiffOp> = Vec::with_capacity(ops.len());
    for op in ops {
        let appended = match (&op, result.last_mut()) {
            (TextDiffOp::Equal(s), Some(TextDiffOp::Equal(acc))) => {
                acc.push_str(s);
                true
            }
            (TextDiffOp::Delete(s), Some(TextDiffOp::Delete(acc))) => {
                acc.push_str(s);
                true
            }
            (TextDiffOp::Insert(s), Some(TextDiffOp::Insert(acc))) => {
                acc.push_str(s);
                true
            }
            _ => false,
        };
        if !appended {
            result.push(op);
        }
    }
    result
}

#[cfg(test)]
mod tests {
    use super::*;

    fn test_snapshot() -> ProjectSnapshotV1 {
        ProjectSnapshotV1 {
            schema_version: SNAPSHOT_SCHEMA_VERSION,
            project: SnapshotProject {
                id: "proj-1".into(),
                title: "Test Project".into(),
                language: "zh-CN".into(),
            },
            commit: SnapshotCommit {
                id: "commit-1".into(),
                root_hash: "sha256:abc".into(),
            },
            branches: vec![SnapshotBranch {
                id: "branch-1".into(),
                name: "main".into(),
                head_commit_id: "commit-1".into(),
            }],
            documents: vec![SnapshotDocument {
                id: "doc-1".into(),
                parent_id: None,
                kind: "chapter".into(),
                title: "Chapter One".into(),
                order_key: "0001".into(),
                revision: 1,
            }],
            blocks: vec![SnapshotBlock {
                id: "block-1".into(),
                document_id: "doc-1".into(),
                kind: "paragraph".into(),
                order_key: "0001".into(),
                content_json: r#"{"kind":"paragraph","text":"hello"}"#.into(),
                plain_text: "hello".into(),
                content_hash: "sha256:def".into(),
                revision: 1,
                locked: false,
            }],
        }
    }

    #[test]
    fn export_json_roundtrip_preserves_project_snapshot() {
        let original = test_snapshot();
        let export = original.to_export();
        let json = serde_json::to_string_pretty(&export).expect("pretty serialize export");
        let parsed: ExportProjectSnapshot =
            serde_json::from_str(&json).expect("deserialize export from pretty json");
        let roundtrip = parsed.to_snapshot();
        assert_eq!(original, roundtrip);
    }

    #[test]
    fn export_json_embeds_content_json_as_object_not_string() {
        let original = test_snapshot();
        let export = original.to_export();
        let json = serde_json::to_string(&export).expect("serialize export");
        let value: serde_json::Value = serde_json::from_str(&json).expect("parse export json");
        let blocks = value["blocks"].as_array().expect("blocks is array");
        let content = &blocks[0]["contentJson"];
        assert!(
            content.is_object(),
            "contentJson must be an object to avoid double-escaping, got: {content}"
        );
        assert!(
            !content.is_string(),
            "contentJson must not be a string in the exported JSON"
        );
    }
}
