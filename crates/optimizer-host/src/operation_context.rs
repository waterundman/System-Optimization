use std::fmt::Write as _;

use optimizer_store::OptimizerStore;
use serde::Serialize;
use sha2::{Digest, Sha256};

use crate::summary_worker::{SummaryContextSpec, summary_context};
use crate::workspace_commands::{
    KnowledgeContextSpec, WorkspaceCommandError, knowledge_context, list_style_samples,
};

const LOCAL_CONTEXT_UTF16_UNITS: usize = 2_000;

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct OperationContextSpec {
    pub base_commit_id: String,
    pub target_block_id: String,
    pub target_block_revision: i64,
    pub target_block_hash: String,
    pub from: usize,
    pub to: usize,
}

#[derive(Debug, Clone, PartialEq, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct OperationContextCandidate {
    pub id: String,
    pub source_ref: String,
    pub source_hash: String,
    pub source_commit_id: String,
    pub tier: String,
    pub status: String,
    pub authority: String,
    pub sensitivity: String,
    pub render_mode: String,
    pub reason_codes: Vec<String>,
    pub content: String,
    pub mandatory: bool,
    pub selected_by_user: bool,
    pub signals: OperationContextSignals,
}

#[derive(Debug, Clone, PartialEq, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct OperationContextSignals {
    pub relevance: f64,
    pub structural_proximity: f64,
    pub freshness: f64,
    pub risk: f64,
}

pub(crate) fn collect_operation_context(
    store: &OptimizerStore,
    project_id: &str,
    spec: &OperationContextSpec,
) -> Result<Vec<OperationContextCandidate>, WorkspaceCommandError> {
    let project = store.get_project(project_id)?;
    if spec.base_commit_id.trim().is_empty() || spec.base_commit_id != project.head_commit_id {
        return Err(WorkspaceCommandError::ContextValidation(
            "Operation context must bind to the current project HEAD".into(),
        ));
    }
    let target = store.get_project_block(project_id, &spec.target_block_id)?;
    if target.revision != spec.target_block_revision
        || target.content_hash != spec.target_block_hash
    {
        return Err(WorkspaceCommandError::ContextValidation(
            "Operation context target changed before collection".into(),
        ));
    }
    if spec.from > spec.to {
        return Err(WorkspaceCommandError::ContextValidation(
            "Operation context selection is reversed".into(),
        ));
    }
    let from_byte = utf16_to_byte(&target.plain_text, spec.from).ok_or_else(|| {
        WorkspaceCommandError::ContextValidation(
            "Operation context start is not a UTF-16 boundary".into(),
        )
    })?;
    let to_byte = utf16_to_byte(&target.plain_text, spec.to).ok_or_else(|| {
        WorkspaceCommandError::ContextValidation(
            "Operation context end is not a UTF-16 boundary".into(),
        )
    })?;

    let documents = store.list_documents(project_id)?;
    let document = documents
        .iter()
        .find(|document| document.id == target.document_id)
        .ok_or_else(|| {
            WorkspaceCommandError::ContextValidation(
                "Operation context target document is not active".into(),
            )
        })?;
    let mut document_blocks = store
        .list_blocks(project_id)?
        .into_iter()
        .filter(|block| block.document_id == target.document_id)
        .collect::<Vec<_>>();
    document_blocks.sort_by(|left, right| {
        left.order_key
            .cmp(&right.order_key)
            .then_with(|| left.id.cmp(&right.id))
    });
    let target_index = document_blocks
        .iter()
        .position(|block| block.id == target.id)
        .ok_or_else(|| {
            WorkspaceCommandError::ContextValidation(
                "Operation context target is not active".into(),
            )
        })?;

    let mut candidates = vec![candidate(
        format!("target-{}-r{}", target.id, target.revision),
        format!(
            "block:{}@r{}#{}-{}",
            target.id, target.revision, spec.from, spec.to
        ),
        &project.head_commit_id,
        "L0_TARGET",
        "canonical",
        "user_confirmed",
        "local_sensitive",
        "verbatim",
        vec!["USER_TARGET".into()],
        target.plain_text[from_byte..to_byte].to_owned(),
        true,
        true,
        signals(1.0, 1.0, 1.0, 0.0),
    )];

    for (index, reason, take_suffix) in [
        (target_index.checked_sub(1), "PREVIOUS_BLOCK", true),
        (target_index.checked_add(1), "NEXT_BLOCK", false),
    ] {
        let Some(block) = index.and_then(|index| document_blocks.get(index)) else {
            continue;
        };
        let content = if take_suffix {
            prefix_window(
                &block.plain_text,
                block.plain_text.len(),
                LOCAL_CONTEXT_UTF16_UNITS,
            )
        } else {
            suffix_window(&block.plain_text, 0, LOCAL_CONTEXT_UTF16_UNITS)
        };
        if content.is_empty() {
            continue;
        }
        candidates.push(candidate(
            format!("local-{}-r{}", block.id, block.revision),
            format!("block:{}@r{}", block.id, block.revision),
            &project.head_commit_id,
            "L1_LOCAL",
            "canonical",
            "source_derived",
            "local_sensitive",
            "verbatim",
            vec![reason.into()],
            content,
            false,
            false,
            signals(0.75, 1.0, 1.0, 0.0),
        ));
    }

    let prefix = prefix_window(&target.plain_text, from_byte, LOCAL_CONTEXT_UTF16_UNITS);
    if !prefix.is_empty() {
        candidates.push(candidate(
            format!("local-{}-prefix-r{}", target.id, target.revision),
            format!("block:{}@r{}#prefix", target.id, target.revision),
            &project.head_commit_id,
            "L1_LOCAL",
            "canonical",
            "source_derived",
            "local_sensitive",
            "verbatim",
            vec!["TARGET_PREFIX".into()],
            prefix,
            false,
            false,
            signals(0.9, 1.0, 1.0, 0.0),
        ));
    }
    let suffix = suffix_window(&target.plain_text, to_byte, LOCAL_CONTEXT_UTF16_UNITS);
    if !suffix.is_empty() {
        candidates.push(candidate(
            format!("local-{}-suffix-r{}", target.id, target.revision),
            format!("block:{}@r{}#suffix", target.id, target.revision),
            &project.head_commit_id,
            "L1_LOCAL",
            "canonical",
            "source_derived",
            "local_sensitive",
            "verbatim",
            vec!["TARGET_SUFFIX".into()],
            suffix,
            false,
            false,
            signals(0.9, 1.0, 1.0, 0.0),
        ));
    }

    candidates.push(candidate(
        format!("structure-{}-r{}", document.id, document.revision),
        format!("document:{}@r{}", document.id, document.revision),
        &project.head_commit_id,
        "L2_STRUCTURAL",
        "canonical",
        "source_derived",
        "local_sensitive",
        "summary",
        vec!["DOCUMENT_STRUCTURE".into()],
        format!(
            "项目：{}\n文档：{}\n文档类型：{}",
            project.title, document.title, document.kind
        ),
        false,
        false,
        signals(0.8, 1.0, 1.0, 0.0),
    ));

    let summary_spec = SummaryContextSpec {
        base_commit_id: spec.base_commit_id.clone(),
        target_block_id: spec.target_block_id.clone(),
        target_block_revision: spec.target_block_revision,
        target_block_hash: spec.target_block_hash.clone(),
    };
    for summary in summary_context(store, project_id, &summary_spec)? {
        let structural = summary.tier == "L2_STRUCTURAL";
        candidates.push(candidate(
            summary.id,
            summary.source_ref,
            &summary.source_commit_id,
            summary.tier,
            summary.status,
            summary.authority,
            summary.sensitivity,
            summary.render_mode,
            summary.reason_codes,
            summary.content,
            false,
            false,
            if structural {
                signals(0.9, 0.9, 1.0, 0.0)
            } else {
                signals(0.65, 0.35, 1.0, 0.0)
            },
        ));
    }

    let knowledge_spec = KnowledgeContextSpec {
        base_commit_id: spec.base_commit_id.clone(),
        target_block_id: spec.target_block_id.clone(),
        target_block_revision: spec.target_block_revision,
        target_block_hash: spec.target_block_hash.clone(),
    };
    for knowledge in knowledge_context(store, project_id, &knowledge_spec)? {
        candidates.push(candidate(
            knowledge.id,
            knowledge.source_ref,
            &knowledge.source_commit_id,
            knowledge.tier,
            knowledge.status,
            knowledge.authority.clone(),
            knowledge.sensitivity,
            knowledge.render_mode,
            knowledge.reason_codes,
            knowledge.content,
            false,
            knowledge.authority == "user_confirmed",
            signals(0.92, 0.5, 1.0, 0.0),
        ));
    }

    for style in list_style_samples(store, project_id)? {
        candidates.push(candidate(
            format!("style-{}-r{}", style.id, style.revision),
            format!("style:{}@r{}", style.id, style.revision),
            &project.head_commit_id,
            "L4_STYLE_GLOBAL",
            style.status.clone(),
            "user_confirmed",
            style.sensitivity,
            "verbatim",
            vec!["PINNED_STYLE_SAMPLE".into()],
            style.content,
            false,
            style.status == "canonical",
            signals(0.8, 0.25, 1.0, 0.0),
        ));
    }
    Ok(candidates)
}

#[allow(clippy::too_many_arguments)]
fn candidate(
    id: String,
    source_ref: String,
    source_commit_id: &str,
    tier: impl Into<String>,
    status: impl Into<String>,
    authority: impl Into<String>,
    sensitivity: impl Into<String>,
    render_mode: impl Into<String>,
    reason_codes: Vec<String>,
    content: String,
    mandatory: bool,
    selected_by_user: bool,
    signals: OperationContextSignals,
) -> OperationContextCandidate {
    OperationContextCandidate {
        id,
        source_ref,
        source_hash: sha256(content.as_bytes()),
        source_commit_id: source_commit_id.to_owned(),
        tier: tier.into(),
        status: status.into(),
        authority: authority.into(),
        sensitivity: sensitivity.into(),
        render_mode: render_mode.into(),
        reason_codes,
        content,
        mandatory,
        selected_by_user,
        signals,
    }
}

fn signals(
    relevance: f64,
    structural_proximity: f64,
    freshness: f64,
    risk: f64,
) -> OperationContextSignals {
    OperationContextSignals {
        relevance,
        structural_proximity,
        freshness,
        risk,
    }
}

fn prefix_window(value: &str, end: usize, maximum_utf16: usize) -> String {
    let prefix = &value[..end];
    let mut start = end;
    let mut used = 0;
    for (byte, character) in prefix.char_indices().rev() {
        let units = character.len_utf16();
        if used + units > maximum_utf16 {
            break;
        }
        used += units;
        start = byte;
    }
    value[start..end].to_owned()
}

fn suffix_window(value: &str, start: usize, maximum_utf16: usize) -> String {
    let suffix = &value[start..];
    let mut end = start;
    let mut used = 0;
    for (byte, character) in suffix.char_indices() {
        let units = character.len_utf16();
        if used + units > maximum_utf16 {
            break;
        }
        used += units;
        end = start + byte + character.len_utf8();
    }
    value[start..end].to_owned()
}

fn utf16_to_byte(value: &str, target: usize) -> Option<usize> {
    if target == 0 {
        return Some(0);
    }
    let mut utf16 = 0;
    for (byte, character) in value.char_indices() {
        if utf16 == target {
            return Some(byte);
        }
        utf16 += character.len_utf16();
        if utf16 > target {
            return None;
        }
    }
    (utf16 == target).then_some(value.len())
}

fn sha256(payload: &[u8]) -> String {
    let mut encoded = String::with_capacity(71);
    encoded.push_str("sha256:");
    for byte in Sha256::digest(payload) {
        let _ = write!(encoded, "{byte:02x}");
    }
    encoded
}

#[cfg(test)]
mod tests {
    use super::{prefix_window, suffix_window, utf16_to_byte};

    #[test]
    fn utf16_windows_never_split_surrogate_pairs() {
        let value = "ab😀中文xyz";
        let end = utf16_to_byte(value, 6).unwrap();
        assert_eq!(&value[..end], "ab😀中文");
        assert_eq!(utf16_to_byte(value, 3), None);
        assert_eq!(prefix_window(value, end, 4), "😀中文");
        assert_eq!(suffix_window(value, 2, 4), "😀中文");
    }
}
