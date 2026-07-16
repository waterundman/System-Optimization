use std::collections::{BTreeMap, BTreeSet};

use optimizer_store::{
    BlockRecord, DocumentRecord, OptimizerStore, PutSummaryRecord, SummaryInvalidationRecord,
    SummaryRecord,
};
use serde::Serialize;
use sha2::{Digest, Sha256};
use time::OffsetDateTime;
use time::format_description::well_known::Rfc3339;

use crate::WorkspaceCommandError;
use crate::workspace_commands::project_root_hash;

const SUMMARY_SCHEMA_VERSION: u32 = 1;
const MAX_REFRESH_BATCH: usize = 64;
const MAX_SOURCE_PREVIEW_CHARS: usize = 32_000;
const BLOCK_SUMMARY_CHARS: usize = 480;
const DOCUMENT_SUMMARY_CHARS: usize = 800;
const PROJECT_SUMMARY_CHARS: usize = 1_200;
const LOCAL_PROVIDER_ID: &str = "optimizer-local";
const LOCAL_MODEL_ID: &str = "extractive-summary-v1";

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct RefreshSummariesSpec {
    pub max_items: usize,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct SummaryRefreshReport {
    pub schema_version: u32,
    pub processed: usize,
    pub remaining: usize,
    pub provider_id: String,
    pub model: String,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct SummaryContextSpec {
    pub base_commit_id: String,
    pub target_block_id: String,
    pub target_block_revision: i64,
    pub target_block_hash: String,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct SummaryContextCandidate {
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
    pub revision: i64,
    pub generated_at: String,
}

struct SummarySource {
    source_hash: String,
    label: String,
    preview: String,
    maximum_summary_chars: usize,
}

pub(crate) fn refresh_summaries(
    store: &mut OptimizerStore,
    project_id: &str,
    spec: &RefreshSummariesSpec,
) -> Result<SummaryRefreshReport, WorkspaceCommandError> {
    if spec.max_items == 0 || spec.max_items > MAX_REFRESH_BATCH {
        return Err(WorkspaceCommandError::Validation(format!(
            "Summary refresh batch must contain 1 to {MAX_REFRESH_BATCH} items"
        )));
    }
    let project = store.get_project(project_id)?;
    let documents = store.list_documents(project_id)?;
    let blocks = store.list_blocks(project_id)?;
    let mut invalidations = store.list_summary_invalidations(project_id, spec.max_items)?;
    invalidations.sort_by(|left, right| {
        summary_scope_rank(&left.scope_type)
            .cmp(&summary_scope_rank(&right.scope_type))
            .then_with(|| left.created_at.cmp(&right.created_at))
            .then_with(|| left.scope_id.cmp(&right.scope_id))
    });

    for invalidation in &invalidations {
        let source = summary_source(invalidation, &project.title, &documents, &blocks)?;
        let summary =
            extractive_summary(&source.label, &source.preview, source.maximum_summary_chars);
        let generated_at = now()?;
        store.put_summary_record(&PutSummaryRecord {
            project_id: project_id.to_owned(),
            scope_type: invalidation.scope_type.clone(),
            scope_id: invalidation.scope_id.clone(),
            expected_source_commit_id: invalidation.source_commit_id.clone(),
            source_hash: source.source_hash,
            summary_hash: sha256(summary.as_bytes()),
            summary,
            provider_id: LOCAL_PROVIDER_ID.into(),
            model: LOCAL_MODEL_ID.into(),
            generated_at,
        })?;
    }

    let remaining = store.list_summary_invalidations(project_id, 1_000)?.len();
    Ok(SummaryRefreshReport {
        schema_version: SUMMARY_SCHEMA_VERSION,
        processed: invalidations.len(),
        remaining,
        provider_id: LOCAL_PROVIDER_ID.into(),
        model: LOCAL_MODEL_ID.into(),
    })
}

pub(crate) fn summary_context(
    store: &OptimizerStore,
    project_id: &str,
    spec: &SummaryContextSpec,
) -> Result<Vec<SummaryContextCandidate>, WorkspaceCommandError> {
    let project = store.get_project(project_id)?;
    if spec.base_commit_id.trim().is_empty() || spec.base_commit_id != project.head_commit_id {
        return Err(WorkspaceCommandError::Validation(
            "Summary context must bind to the current project HEAD".into(),
        ));
    }
    let target = store.get_project_block(project_id, &spec.target_block_id)?;
    if target.revision != spec.target_block_revision
        || target.content_hash != spec.target_block_hash
    {
        return Err(WorkspaceCommandError::Validation(
            "Summary context target changed before collection".into(),
        ));
    }
    let documents = store.list_documents(project_id)?;
    let by_id = documents
        .iter()
        .map(|document| (document.id.as_str(), document))
        .collect::<BTreeMap<_, _>>();
    let mut candidates = Vec::new();
    let mut cursor = Some(target.document_id.as_str());
    let mut depth = 0usize;
    let mut visited = BTreeSet::new();
    while let Some(document_id) = cursor {
        if !visited.insert(document_id) {
            return Err(WorkspaceCommandError::Validation(
                "Document hierarchy contains a cycle".into(),
            ));
        }
        let document = by_id.get(document_id).ok_or_else(|| {
            WorkspaceCommandError::Validation(
                "Summary context target document is not active".into(),
            )
        })?;
        if let Some(record) = store.get_ready_summary_record(project_id, "document", document_id)? {
            candidates.push(context_candidate(
                record,
                "L2_STRUCTURAL",
                if depth == 0 {
                    "CURRENT_DOCUMENT_SUMMARY"
                } else {
                    "ANCESTOR_DOCUMENT_SUMMARY"
                },
            ));
        }
        cursor = document.parent_id.as_deref();
        depth += 1;
    }
    if let Some(record) = store.get_ready_summary_record(project_id, "project", project_id)? {
        candidates.push(context_candidate(record, "L3_KNOWLEDGE", "PROJECT_SUMMARY"));
    }
    Ok(candidates)
}

fn summary_source(
    invalidation: &SummaryInvalidationRecord,
    project_title: &str,
    documents: &[DocumentRecord],
    blocks: &[BlockRecord],
) -> Result<SummarySource, WorkspaceCommandError> {
    match invalidation.scope_type.as_str() {
        "block" => {
            let block = blocks
                .iter()
                .find(|block| block.id == invalidation.scope_id)
                .ok_or_else(|| {
                    WorkspaceCommandError::Validation(
                        "Summary invalidation references an inactive Block".into(),
                    )
                })?;
            Ok(SummarySource {
                source_hash: block.content_hash.clone(),
                label: "正文块摘要".into(),
                preview: limited_text(&block.plain_text, MAX_SOURCE_PREVIEW_CHARS),
                maximum_summary_chars: BLOCK_SUMMARY_CHARS,
            })
        }
        "document" => {
            let root = documents
                .iter()
                .find(|document| document.id == invalidation.scope_id)
                .ok_or_else(|| {
                    WorkspaceCommandError::Validation(
                        "Summary invalidation references an inactive Document".into(),
                    )
                })?;
            let ordered = ordered_subtree(documents, &root.id);
            let ids = ordered
                .iter()
                .map(|document| document.id.as_str())
                .collect::<BTreeSet<_>>();
            let source_hash = project_root_hash(
                ordered.iter().copied(),
                blocks
                    .iter()
                    .filter(|block| ids.contains(block.document_id.as_str()))
                    .map(|block| (block.id.as_str(), block.content_hash.as_str())),
            );
            let mut preview = format!("章节：{}\n结构：", root.title);
            for (index, document) in ordered.iter().enumerate() {
                if index > 0 {
                    preview.push_str(" > ");
                }
                preview.push_str(&document.title);
            }
            preview.push('\n');
            append_block_previews(&mut preview, &ordered, blocks);
            Ok(SummarySource {
                source_hash,
                label: format!("章节“{}”摘要", root.title),
                preview: limited_text(&preview, MAX_SOURCE_PREVIEW_CHARS),
                maximum_summary_chars: DOCUMENT_SUMMARY_CHARS,
            })
        }
        "project" => {
            if invalidation.scope_id != invalidation.project_id {
                return Err(WorkspaceCommandError::Validation(
                    "Project summary scope must use the project ID".into(),
                ));
            }
            let ordered = ordered_forest(documents);
            let source_hash = project_root_hash(
                documents.iter(),
                blocks
                    .iter()
                    .map(|block| (block.id.as_str(), block.content_hash.as_str())),
            );
            let mut preview = format!("项目：{project_title}\n结构：");
            for (index, document) in ordered.iter().enumerate() {
                if index > 0 {
                    preview.push_str(" / ");
                }
                preview.push_str(&document.title);
            }
            preview.push('\n');
            append_block_previews(&mut preview, &ordered, blocks);
            Ok(SummarySource {
                source_hash,
                label: format!("项目“{project_title}”摘要"),
                preview: limited_text(&preview, MAX_SOURCE_PREVIEW_CHARS),
                maximum_summary_chars: PROJECT_SUMMARY_CHARS,
            })
        }
        _ => Err(WorkspaceCommandError::Validation(
            "Summary invalidation scope is unsupported".into(),
        )),
    }
}

fn context_candidate(
    record: SummaryRecord,
    tier: &str,
    reason_code: &str,
) -> SummaryContextCandidate {
    SummaryContextCandidate {
        id: format!(
            "summary-{}-{}-r{}",
            record.scope_type, record.scope_id, record.revision
        ),
        source_ref: format!(
            "summary:{}:{}@{}",
            record.scope_type, record.scope_id, record.source_commit_id
        ),
        source_hash: record.summary_hash,
        source_commit_id: record.source_commit_id,
        tier: tier.into(),
        status: "canonical".into(),
        authority: if record.provider_id == LOCAL_PROVIDER_ID {
            "source_derived".into()
        } else {
            "model_inferred".into()
        },
        sensitivity: "local_sensitive".into(),
        render_mode: "summary".into(),
        reason_codes: vec![reason_code.into()],
        content: record.summary,
        revision: record.revision,
        generated_at: record.generated_at,
    }
}

fn summary_scope_rank(scope_type: &str) -> u8 {
    match scope_type {
        "block" => 0,
        "document" => 1,
        "project" => 2,
        _ => u8::MAX,
    }
}

fn ordered_forest(documents: &[DocumentRecord]) -> Vec<&DocumentRecord> {
    let mut roots = documents
        .iter()
        .filter(|document| document.parent_id.is_none())
        .collect::<Vec<_>>();
    sort_documents(&mut roots);
    let mut ordered = Vec::with_capacity(documents.len());
    let mut visited = BTreeSet::new();
    for root in roots {
        visit_document(documents, root, &mut visited, &mut ordered);
    }
    for document in documents {
        if !visited.contains(document.id.as_str()) {
            visit_document(documents, document, &mut visited, &mut ordered);
        }
    }
    ordered
}

fn ordered_subtree<'a>(documents: &'a [DocumentRecord], root_id: &str) -> Vec<&'a DocumentRecord> {
    let Some(root) = documents.iter().find(|document| document.id == root_id) else {
        return Vec::new();
    };
    let mut ordered = Vec::new();
    let mut visited = BTreeSet::new();
    visit_document(documents, root, &mut visited, &mut ordered);
    ordered
}

fn visit_document<'a>(
    documents: &'a [DocumentRecord],
    document: &'a DocumentRecord,
    visited: &mut BTreeSet<&'a str>,
    ordered: &mut Vec<&'a DocumentRecord>,
) {
    if !visited.insert(document.id.as_str()) {
        return;
    }
    ordered.push(document);
    let mut children = documents
        .iter()
        .filter(|candidate| candidate.parent_id.as_deref() == Some(document.id.as_str()))
        .collect::<Vec<_>>();
    sort_documents(&mut children);
    for child in children {
        visit_document(documents, child, visited, ordered);
    }
}

fn sort_documents(documents: &mut Vec<&DocumentRecord>) {
    documents.sort_by(|left, right| {
        left.order_key
            .cmp(&right.order_key)
            .then_with(|| left.id.cmp(&right.id))
    });
}

fn append_block_previews(
    output: &mut String,
    documents: &[&DocumentRecord],
    blocks: &[BlockRecord],
) {
    for document in documents {
        if output.chars().count() >= MAX_SOURCE_PREVIEW_CHARS {
            break;
        }
        output.push_str("\n[");
        output.push_str(&document.title);
        output.push_str("] ");
        for block in blocks
            .iter()
            .filter(|block| block.document_id == document.id)
        {
            let used = output.chars().count();
            if used >= MAX_SOURCE_PREVIEW_CHARS {
                return;
            }
            output.extend(
                block
                    .plain_text
                    .chars()
                    .take((MAX_SOURCE_PREVIEW_CHARS - used).min(1_200)),
            );
            output.push(' ');
        }
    }
}

fn extractive_summary(label: &str, source: &str, maximum_chars: usize) -> String {
    let normalized = normalize_whitespace(source);
    if normalized.is_empty() {
        return format!("{label}：暂无正文。");
    }
    let mut body = normalized.chars().take(maximum_chars).collect::<String>();
    if normalized.chars().count() > maximum_chars {
        while body.chars().last().is_some_and(|character| {
            matches!(character, ' ' | '，' | ',' | '；' | ';' | '：' | ':')
        }) {
            body.pop();
        }
        body.push('…');
    }
    format!("{label}：{body}")
}

fn normalize_whitespace(value: &str) -> String {
    let mut output = String::with_capacity(value.len());
    let mut whitespace = false;
    for character in value.chars() {
        if character.is_whitespace() {
            whitespace = !output.is_empty();
        } else {
            if whitespace {
                output.push(' ');
                whitespace = false;
            }
            output.push(character);
        }
    }
    output
}

fn limited_text(value: &str, maximum_chars: usize) -> String {
    value.chars().take(maximum_chars).collect()
}

fn sha256(payload: &[u8]) -> String {
    let digest = Sha256::digest(payload);
    format!("sha256:{digest:x}")
}

fn now() -> Result<String, WorkspaceCommandError> {
    OffsetDateTime::now_utc()
        .format(&Rfc3339)
        .map_err(|_| WorkspaceCommandError::Clock)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn extractive_summary_is_bounded_and_deterministic() {
        let source = " 第一段。\n\n第二段包含更多内容。 ";
        assert_eq!(
            extractive_summary("测试", source, 8),
            "测试：第一段。 第二段…"
        );
        assert_eq!(extractive_summary("测试", "  ", 8), "测试：暂无正文。");
    }
}
