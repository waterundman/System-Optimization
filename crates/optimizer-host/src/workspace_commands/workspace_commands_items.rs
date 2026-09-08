use optimizer_store::{
    ApplyBlockEdit, CreateKnowledgeItem, CreateStyleSample, KnowledgeItemRecord,
    SetKnowledgeItemStatus, SetStyleSampleStatus, StyleSampleRecord,
};

use super::*;

pub(crate) fn list_style_samples(
    store: &OptimizerStore,
    project_id: &str,
) -> Result<Vec<StyleSample>, WorkspaceCommandError> {
    store
        .list_style_samples(project_id)?
        .into_iter()
        .map(style_sample)
        .collect::<Result<Vec<_>, _>>()
}

pub(crate) fn create_style_sample(
    store: &mut OptimizerStore,
    project_id: &str,
    spec: &CreateStyleSampleSpec,
) -> Result<StyleSample, WorkspaceCommandError> {
    validate_style_sample(spec)?;
    let title = spec.title.trim().to_owned();
    let content = spec.content.trim().to_owned();
    let content_hash = sha256(content.as_bytes());
    let record = store.create_style_sample(&CreateStyleSample {
        id: generated_id("style"),
        project_id: project_id.to_owned(),
        title,
        content,
        content_hash,
        sensitivity: spec.sensitivity.clone(),
        created_at: now()?,
    })?;
    style_sample(record)
}

pub(crate) fn set_style_sample_status(
    store: &mut OptimizerStore,
    project_id: &str,
    spec: &SetStyleSampleStatusSpec,
) -> Result<StyleSample, WorkspaceCommandError> {
    if spec.id.trim().is_empty() || spec.expected_revision < 0 {
        return Err(WorkspaceCommandError::StyleValidation(
            "Style sample id and non-negative revision are required".into(),
        ));
    }
    if !matches!(spec.status.as_str(), "canonical" | "archived") {
        return Err(WorkspaceCommandError::StyleValidation(
            "Style sample status must be canonical or archived".into(),
        ));
    }
    let record = store.set_style_sample_status(&SetStyleSampleStatus {
        project_id: project_id.to_owned(),
        id: spec.id.clone(),
        expected_revision: spec.expected_revision,
        status: spec.status.clone(),
        updated_at: now()?,
    })?;
    style_sample(record)
}

pub(crate) fn list_knowledge_items(
    store: &OptimizerStore,
    project_id: &str,
) -> Result<Vec<KnowledgeItem>, WorkspaceCommandError> {
    store
        .list_knowledge_items(project_id)?
        .into_iter()
        .map(knowledge_item)
        .collect::<Result<Vec<_>, _>>()
}

pub(crate) fn create_knowledge_item(
    store: &mut OptimizerStore,
    project_id: &str,
    spec: &CreateKnowledgeItemSpec,
) -> Result<KnowledgeItem, WorkspaceCommandError> {
    validate_knowledge_item(spec)?;
    let title = spec.title.trim().to_owned();
    let content = spec.content.trim().to_owned();
    let record = store.create_knowledge_item(&CreateKnowledgeItem {
        id: generated_id("knowledge"),
        project_id: project_id.to_owned(),
        kind: spec.kind.clone(),
        title,
        content_hash: sha256(content.as_bytes()),
        content,
        authority: "user_confirmed".into(),
        sensitivity: spec.sensitivity.clone(),
        severity: spec.severity.clone(),
        created_at: now()?,
    })?;
    knowledge_item(record)
}

pub(crate) fn set_knowledge_item_status(
    store: &mut OptimizerStore,
    project_id: &str,
    spec: &SetKnowledgeItemStatusSpec,
) -> Result<KnowledgeItem, WorkspaceCommandError> {
    if spec.id.trim().is_empty() || spec.expected_revision < 0 {
        return Err(WorkspaceCommandError::KnowledgeValidation(
            "Knowledge item id and non-negative revision are required".into(),
        ));
    }
    if !matches!(spec.status.as_str(), "canonical" | "archived" | "rejected") {
        return Err(WorkspaceCommandError::KnowledgeValidation(
            "Knowledge item status must be canonical, archived or rejected".into(),
        ));
    }
    let record = store.set_knowledge_item_status(&SetKnowledgeItemStatus {
        project_id: project_id.to_owned(),
        id: spec.id.clone(),
        expected_revision: spec.expected_revision,
        status: spec.status.clone(),
        updated_at: now()?,
    })?;
    knowledge_item(record)
}

pub(crate) fn knowledge_context(
    store: &OptimizerStore,
    project_id: &str,
    spec: &KnowledgeContextSpec,
) -> Result<Vec<KnowledgeContextCandidate>, WorkspaceCommandError> {
    let project = store.get_project(project_id)?;
    if spec.base_commit_id.trim().is_empty() || spec.base_commit_id != project.head_commit_id {
        return Err(WorkspaceCommandError::KnowledgeValidation(
            "Knowledge context must bind to the current project HEAD".into(),
        ));
    }
    let target = store.get_project_block(project_id, &spec.target_block_id)?;
    if target.revision != spec.target_block_revision
        || target.content_hash != spec.target_block_hash
    {
        return Err(WorkspaceCommandError::KnowledgeValidation(
            "Knowledge context target changed before collection".into(),
        ));
    }
    store
        .list_knowledge_items(project_id)?
        .into_iter()
        .filter(|record| record.status == "canonical")
        .map(|record| knowledge_context_candidate(record, &project.head_commit_id))
        .collect()
}

pub(crate) fn save_block(
    store: &mut OptimizerStore,
    project_id: &str,
    main_branch_id: &str,
    spec: &SaveBlockSpec,
) -> Result<SaveBlockResponse, WorkspaceCommandError> {
    validate_save_spec(spec)?;
    let project = store.get_project(project_id)?;
    let current = store.get_project_block(project_id, &spec.block_id)?;
    if current.revision != spec.expected_revision || current.content_hash != spec.expected_hash {
        return Err(WorkspaceCommandError::Store(StoreError::Conflict {
            entity: "block",
            id: current.id,
            expected_revision: spec.expected_revision,
            actual_revision: current.revision,
            expected_hash: spec.expected_hash.clone(),
            actual_hash: current.content_hash,
        }));
    }
    let content_json = serde_json::to_string(&spec.content).map_err(WorkspaceCommandError::Json)?;
    if content_json.len() > MAX_CONTENT_JSON_BYTES {
        return Err(WorkspaceCommandError::Validation(format!(
            "Block content exceeds {MAX_CONTENT_JSON_BYTES} bytes"
        )));
    }
    let content_hash = block_content_hash(
        &current.kind,
        &spec.content,
        &spec.plain_text,
        current.locked,
    )?;
    if content_hash == current.content_hash
        && content_json == current.content_json
        && spec.plain_text == current.plain_text
    {
        return Err(WorkspaceCommandError::NoChanges);
    }
    let documents = store.list_documents(project_id)?;
    let blocks = store.list_blocks(project_id)?;
    let root_hash = project_root_hash(
        documents.iter(),
        blocks.iter().map(|block| {
            (
                block.id.as_str(),
                if block.id == current.id {
                    content_hash.as_str()
                } else {
                    block.content_hash.as_str()
                },
            )
        }),
    );
    let command = ApplyBlockEdit {
        edit_id: generated_id("edit"),
        commit_id: generated_id("commit"),
        branch_id: main_branch_id.into(),
        expected_head_commit_id: project.head_commit_id,
        expected_project_revision: project.revision,
        block_id: spec.block_id.clone(),
        expected_revision: spec.expected_revision,
        expected_hash: spec.expected_hash.clone(),
        new_content_json: content_json,
        new_plain_text: spec.plain_text.clone(),
        new_content_hash: content_hash,
        new_root_hash: root_hash,
        reason: "autosave".into(),
        actor_type: "user".into(),
        actor_id: None,
        occurred_at: now()?,
        review_event: None,
    };
    let receipt = store.apply_block_edit(&command)?;
    save_response(store, project_id, receipt)
}

fn style_sample(record: StyleSampleRecord) -> Result<StyleSample, WorkspaceCommandError> {
    if record.id.trim().is_empty()
        || record.title.trim().is_empty()
        || record.content.trim().is_empty()
        || record.revision < 0
        || record.content_hash != sha256(record.content.as_bytes())
        || !matches!(record.status.as_str(), "canonical" | "archived")
        || !matches!(
            record.sensitivity.as_str(),
            "local_sensitive" | "never_send"
        )
    {
        return Err(WorkspaceCommandError::Store(
            StoreError::InvariantViolation("stored style sample policy is invalid".into()),
        ));
    }
    Ok(StyleSample {
        schema_version: WORKSPACE_SCHEMA_VERSION,
        id: record.id,
        title: record.title,
        content: record.content,
        content_hash: record.content_hash,
        status: record.status,
        sensitivity: record.sensitivity,
        revision: record.revision,
        created_at: record.created_at,
        updated_at: record.updated_at,
    })
}

fn knowledge_item(record: KnowledgeItemRecord) -> Result<KnowledgeItem, WorkspaceCommandError> {
    validate_stored_knowledge_item(&record)?;
    Ok(KnowledgeItem {
        schema_version: WORKSPACE_SCHEMA_VERSION,
        id: record.id,
        kind: record.kind,
        title: record.title,
        content: record.content,
        content_hash: record.content_hash,
        status: record.status,
        authority: record.authority,
        sensitivity: record.sensitivity,
        severity: record.severity,
        revision: record.revision,
        created_at: record.created_at,
        updated_at: record.updated_at,
    })
}

fn knowledge_context_candidate(
    record: KnowledgeItemRecord,
    source_commit_id: &str,
) -> Result<KnowledgeContextCandidate, WorkspaceCommandError> {
    validate_stored_knowledge_item(&record)?;
    let label = match (record.kind.as_str(), record.severity.as_deref()) {
        ("fact", None) => "事实",
        ("constraint", Some("hard")) => "硬约束",
        ("constraint", Some("soft")) => "软约束",
        _ => {
            return Err(WorkspaceCommandError::Store(
                StoreError::InvariantViolation("stored knowledge type is invalid".into()),
            ));
        }
    };
    let content = format!("{label}【{}】：{}", record.title, record.content);
    let reason_code = match (record.kind.as_str(), record.severity.as_deref()) {
        ("fact", None) => "CANONICAL_FACT",
        ("constraint", Some("hard")) => "PROJECT_HARD_CONSTRAINT",
        ("constraint", Some("soft")) => "PROJECT_SOFT_CONSTRAINT",
        _ => {
            return Err(invariant_violation(
                "stored knowledge policy failed validation",
            ));
        }
    };
    Ok(KnowledgeContextCandidate {
        id: format!("knowledge-{}-r{}", record.id, record.revision),
        source_ref: format!(
            "knowledge:{}:{}@r{}",
            record.kind, record.id, record.revision
        ),
        source_hash: sha256(content.as_bytes()),
        source_commit_id: source_commit_id.to_owned(),
        tier: "L3_KNOWLEDGE".into(),
        status: record.status,
        authority: record.authority,
        sensitivity: record.sensitivity,
        render_mode: "constraint".into(),
        reason_codes: vec![reason_code.into()],
        content,
        revision: record.revision,
        generated_at: record.updated_at,
    })
}

fn validate_save_spec(spec: &SaveBlockSpec) -> Result<(), WorkspaceCommandError> {
    if spec.block_id.trim().is_empty()
        || spec.block_id.len() > 200
        || spec.expected_hash.trim().is_empty()
        || spec.expected_revision < 0
    {
        return Err(WorkspaceCommandError::Validation(
            "Block id, expected revision and expected hash are invalid".into(),
        ));
    }
    if !spec.content.is_object() {
        return Err(WorkspaceCommandError::Validation(
            "Block content must be a JSON object".into(),
        ));
    }
    if spec.plain_text.len() > MAX_PLAIN_TEXT_BYTES {
        return Err(WorkspaceCommandError::Validation(format!(
            "Block plain text exceeds {MAX_PLAIN_TEXT_BYTES} bytes"
        )));
    }
    Ok(())
}

fn validate_style_sample(spec: &CreateStyleSampleSpec) -> Result<(), WorkspaceCommandError> {
    let title = spec.title.trim();
    let content = spec.content.trim();
    if title.is_empty() || title.chars().count() > MAX_STYLE_TITLE_CHARS {
        return Err(WorkspaceCommandError::StyleValidation(format!(
            "Style sample title must contain 1 to {MAX_STYLE_TITLE_CHARS} characters"
        )));
    }
    if content.is_empty() || content.len() > MAX_STYLE_SAMPLE_BYTES {
        return Err(WorkspaceCommandError::StyleValidation(format!(
            "Style sample content must contain 1 to {MAX_STYLE_SAMPLE_BYTES} bytes"
        )));
    }
    if !matches!(spec.sensitivity.as_str(), "local_sensitive" | "never_send") {
        return Err(WorkspaceCommandError::StyleValidation(
            "Style sample sensitivity must be local_sensitive or never_send".into(),
        ));
    }
    Ok(())
}

fn validate_knowledge_item(spec: &CreateKnowledgeItemSpec) -> Result<(), WorkspaceCommandError> {
    let title = spec.title.trim();
    let content = spec.content.trim();
    if title.is_empty() || title.chars().count() > MAX_KNOWLEDGE_TITLE_CHARS {
        return Err(WorkspaceCommandError::KnowledgeValidation(format!(
            "Knowledge title must contain 1 to {MAX_KNOWLEDGE_TITLE_CHARS} characters"
        )));
    }
    if content.is_empty() || content.len() > MAX_KNOWLEDGE_CONTENT_BYTES {
        return Err(WorkspaceCommandError::KnowledgeValidation(format!(
            "Knowledge content must contain 1 to {MAX_KNOWLEDGE_CONTENT_BYTES} bytes"
        )));
    }
    if !matches!(spec.kind.as_str(), "fact" | "constraint") {
        return Err(WorkspaceCommandError::KnowledgeValidation(
            "Knowledge kind must be fact or constraint".into(),
        ));
    }
    let valid_severity = match spec.kind.as_str() {
        "fact" => spec.severity.is_none(),
        "constraint" => matches!(spec.severity.as_deref(), Some("hard" | "soft")),
        _ => false,
    };
    if !valid_severity {
        return Err(WorkspaceCommandError::KnowledgeValidation(
            "Facts have no severity; constraints require hard or soft severity".into(),
        ));
    }
    if !matches!(
        spec.sensitivity.as_str(),
        "public" | "local" | "local_sensitive" | "never_send"
    ) {
        return Err(WorkspaceCommandError::KnowledgeValidation(
            "Knowledge sensitivity is unsupported".into(),
        ));
    }
    Ok(())
}

fn validate_stored_knowledge_item(
    record: &KnowledgeItemRecord,
) -> Result<(), WorkspaceCommandError> {
    let identity_valid = !record.id.trim().is_empty()
        && !record.project_id.trim().is_empty()
        && !record.title.trim().is_empty()
        && !record.content.trim().is_empty()
        && !record.created_at.trim().is_empty()
        && !record.updated_at.trim().is_empty()
        && record.revision >= 0;
    let hash_valid = record.content_hash == sha256(record.content.as_bytes());
    let status_valid = matches!(
        record.status.as_str(),
        "canonical" | "archived" | "rejected"
    );
    let authority_valid = matches!(
        record.authority.as_str(),
        "user_confirmed" | "source_derived" | "model_inferred" | "external_untrusted"
    );
    let sensitivity_valid = matches!(
        record.sensitivity.as_str(),
        "public" | "local" | "local_sensitive" | "never_send"
    );
    let type_valid = match record.kind.as_str() {
        "fact" => record.severity.is_none(),
        "constraint" => matches!(record.severity.as_deref(), Some("hard" | "soft")),
        _ => false,
    };
    if !identity_valid
        || !hash_valid
        || !status_valid
        || !authority_valid
        || !sensitivity_valid
        || !type_valid
    {
        return Err(WorkspaceCommandError::Store(
            StoreError::InvariantViolation("stored knowledge policy is invalid".into()),
        ));
    }
    Ok(())
}
