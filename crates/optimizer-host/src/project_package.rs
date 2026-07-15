use std::fmt;
use std::fs;
use std::path::{Component, Path};

use optimizer_store::{
    OptimizerStore, ProjectRecord, ProjectSeed, SeedBlock, SeedDocument, StoreError,
};
use serde::{Deserialize, Serialize};
use time::OffsetDateTime;
use time::format_description::well_known::Rfc3339;
use uuid::Uuid;

use crate::workspace_commands::{
    apply_reviewed_proposal, block_content_hash, create_checkpoint, create_document,
    create_style_sample, list_style_samples, load_project_workspace, load_version_history,
    project_root_hash, restore_checkpoint, save_block, set_style_sample_status,
};
use crate::{
    ApplyReviewedProposalResponse, ApplyReviewedProposalSpec, CheckpointSummary,
    CreateDocumentResponse, CreateDocumentSpec, CreateStyleSampleSpec, HostError,
    OperationCommandHost, ProjectRoot, ProjectWorkspace, RestoreCheckpointResponse,
    RestoreCheckpointSpec, SaveBlockResponse, SaveBlockSpec, SetStyleSampleStatusSpec, StyleSample,
    VersionHistory, WorkspaceCommandError,
};

const PACKAGE_SCHEMA_VERSION: u32 = 1;
const DATABASE_FILE: &str = "project.sqlite3";
const MANIFEST_FILE: &str = "manifest.json";
const MAX_MANIFEST_BYTES: u64 = 64 * 1024;

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct NewProjectSpec {
    pub folder_name: String,
    pub title: String,
    pub language: String,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
pub struct ProjectPackageManifest {
    pub schema_version: u32,
    pub project_id: String,
    pub main_branch_id: String,
    pub title: String,
    pub language: String,
    pub database: String,
    pub created_at: String,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct ProjectInfo {
    pub schema_version: u32,
    pub project_id: String,
    pub main_branch_id: String,
    pub title: String,
    pub language: String,
    pub directory: String,
    pub database_schema_version: i64,
    pub head_commit_id: String,
    pub revision: i64,
    pub created_at: String,
    pub updated_at: String,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct ExportMarkdownResponse {
    pub schema_version: u32,
    pub path: String,
    pub bytes: usize,
    pub documents: usize,
}

pub struct OpenedProject {
    root: ProjectRoot,
    project_id: String,
    main_branch_id: String,
    database_schema_version: i64,
    operations: OperationCommandHost,
}

impl OpenedProject {
    pub fn create(
        parent_directory: impl AsRef<Path>,
        spec: &NewProjectSpec,
    ) -> Result<Self, ProjectPackageError> {
        validate_spec(spec)?;
        let parent = ProjectRoot::open(parent_directory).map_err(ProjectPackageError::Host)?;
        let final_path = parent
            .resolve_for_create(&spec.folder_name)
            .map_err(ProjectPackageError::Host)?;
        if final_path.exists() {
            return Err(ProjectPackageError::AlreadyExists);
        }

        let staging_name = format!(".{}.creating-{}", spec.folder_name, Uuid::new_v4().simple());
        let staging_path = parent
            .resolve_for_create(&staging_name)
            .map_err(ProjectPackageError::Host)?;
        fs::create_dir(&staging_path).map_err(|source| ProjectPackageError::Io {
            operation: "create staging directory",
            source,
        })?;

        if let Err(error) = create_staged_package(&staging_path, spec) {
            cleanup_staging_directory(&parent, &staging_path);
            return Err(error);
        }
        if let Err(source) = fs::rename(&staging_path, &final_path) {
            cleanup_staging_directory(&parent, &staging_path);
            return Err(ProjectPackageError::Io {
                operation: "publish project package",
                source,
            });
        }
        Self::open(final_path)
    }

    pub fn open(project_directory: impl AsRef<Path>) -> Result<Self, ProjectPackageError> {
        let root = ProjectRoot::open(project_directory).map_err(ProjectPackageError::Host)?;
        let manifest_path = root
            .resolve_existing(MANIFEST_FILE)
            .map_err(|_| ProjectPackageError::InvalidPackage("manifest is missing"))?;
        let metadata = fs::metadata(&manifest_path).map_err(|source| ProjectPackageError::Io {
            operation: "inspect project manifest",
            source,
        })?;
        if metadata.len() == 0 || metadata.len() > MAX_MANIFEST_BYTES {
            return Err(ProjectPackageError::InvalidPackage(
                "manifest size is invalid",
            ));
        }
        let manifest: ProjectPackageManifest =
            serde_json::from_slice(&fs::read(&manifest_path).map_err(|source| {
                ProjectPackageError::Io {
                    operation: "read project manifest",
                    source,
                }
            })?)
            .map_err(ProjectPackageError::ManifestJson)?;
        validate_manifest(&manifest)?;

        let database_path = root
            .resolve_existing(&manifest.database)
            .map_err(|_| ProjectPackageError::InvalidPackage("database is missing"))?;
        let store = OptimizerStore::open(database_path).map_err(ProjectPackageError::Store)?;
        store
            .verify_invariants()
            .map_err(ProjectPackageError::Store)?;
        let record = store
            .get_project(&manifest.project_id)
            .map_err(ProjectPackageError::Store)?;
        let branch = store
            .get_branch(&manifest.main_branch_id)
            .map_err(ProjectPackageError::Store)?;
        if record.id != manifest.project_id
            || branch.project_id != record.id
            || branch.name != "main"
            || branch.head_commit_id != record.head_commit_id
        {
            return Err(ProjectPackageError::InvalidPackage(
                "manifest project binding is invalid",
            ));
        }
        let database_schema_version = store
            .diagnostics()
            .map_err(ProjectPackageError::Store)?
            .schema_version;
        Ok(Self {
            root,
            project_id: record.id,
            main_branch_id: branch.id,
            database_schema_version,
            operations: OperationCommandHost::new(store),
        })
    }

    pub fn info(&self) -> Result<ProjectInfo, ProjectPackageError> {
        let record = self
            .operations
            .get_project_record(&self.project_id)
            .map_err(ProjectPackageError::Operation)?;
        Ok(project_info(
            &self.root,
            &self.main_branch_id,
            self.database_schema_version,
            record,
        ))
    }

    pub fn export_markdown(&self) -> Result<ExportMarkdownResponse, ProjectPackageError> {
        let project = self
            .operations
            .store()
            .get_project(&self.project_id)
            .map_err(ProjectPackageError::Store)?;
        let documents = self
            .operations
            .store()
            .list_documents(&self.project_id)
            .map_err(ProjectPackageError::Store)?;
        let blocks = self
            .operations
            .store()
            .list_blocks(&self.project_id)
            .map_err(ProjectPackageError::Store)?;
        let mut markdown = String::new();
        markdown.push_str("# ");
        markdown.push_str(&project.title);
        markdown.push_str("\n\n");
        for document in &documents {
            markdown.push_str("## ");
            markdown.push_str(&document.title);
            markdown.push_str("\n\n");
            for block in blocks
                .iter()
                .filter(|block| block.document_id == document.id)
            {
                markdown.push_str(&block.plain_text);
                markdown.push_str("\n\n");
            }
        }

        let exports = self
            .root
            .resolve_for_create("exports")
            .map_err(ProjectPackageError::Host)?;
        fs::create_dir_all(&exports).map_err(|source| ProjectPackageError::Io {
            operation: "create exports directory",
            source,
        })?;
        let file_name = format!("optimizer-export-{}.md", Uuid::new_v4().simple());
        let relative = Path::new("exports").join(&file_name);
        let final_path = self
            .root
            .resolve_for_create(&relative)
            .map_err(ProjectPackageError::Host)?;
        let temporary = self
            .root
            .resolve_for_create(Path::new("exports").join(format!(".{file_name}.tmp")))
            .map_err(ProjectPackageError::Host)?;
        fs::write(&temporary, markdown.as_bytes()).map_err(|source| ProjectPackageError::Io {
            operation: "write Markdown export",
            source,
        })?;
        if let Err(source) = fs::rename(&temporary, &final_path) {
            let _ = fs::remove_file(&temporary);
            return Err(ProjectPackageError::Io {
                operation: "publish Markdown export",
                source,
            });
        }
        Ok(ExportMarkdownResponse {
            schema_version: 1,
            path: final_path.to_string_lossy().into_owned(),
            bytes: markdown.len(),
            documents: documents.len(),
        })
    }

    pub fn operations(&self) -> &OperationCommandHost {
        &self.operations
    }

    pub fn operations_mut(&mut self) -> &mut OperationCommandHost {
        &mut self.operations
    }

    pub fn workspace(&self) -> Result<ProjectWorkspace, WorkspaceCommandError> {
        load_project_workspace(
            self.operations.store(),
            &self.project_id,
            &self.main_branch_id,
        )
    }

    pub fn create_document(
        &mut self,
        spec: &CreateDocumentSpec,
    ) -> Result<CreateDocumentResponse, WorkspaceCommandError> {
        create_document(
            self.operations.store_mut(),
            &self.project_id,
            &self.main_branch_id,
            spec,
        )
    }

    pub fn style_samples(&self) -> Result<Vec<StyleSample>, WorkspaceCommandError> {
        list_style_samples(self.operations.store(), &self.project_id)
    }

    pub fn create_style_sample(
        &mut self,
        spec: &CreateStyleSampleSpec,
    ) -> Result<StyleSample, WorkspaceCommandError> {
        create_style_sample(self.operations.store_mut(), &self.project_id, spec)
    }

    pub fn set_style_sample_status(
        &mut self,
        spec: &SetStyleSampleStatusSpec,
    ) -> Result<StyleSample, WorkspaceCommandError> {
        set_style_sample_status(self.operations.store_mut(), &self.project_id, spec)
    }

    pub fn save_block(
        &mut self,
        spec: &SaveBlockSpec,
    ) -> Result<SaveBlockResponse, WorkspaceCommandError> {
        save_block(
            self.operations.store_mut(),
            &self.project_id,
            &self.main_branch_id,
            spec,
        )
    }

    pub fn apply_reviewed_proposal(
        &mut self,
        spec: &ApplyReviewedProposalSpec,
    ) -> Result<ApplyReviewedProposalResponse, WorkspaceCommandError> {
        apply_reviewed_proposal(
            self.operations.store_mut(),
            &self.project_id,
            &self.main_branch_id,
            spec,
        )
    }

    pub fn create_checkpoint(&mut self) -> Result<CheckpointSummary, WorkspaceCommandError> {
        create_checkpoint(self.operations.store_mut(), &self.project_id)
    }

    pub fn version_history(&self) -> Result<VersionHistory, WorkspaceCommandError> {
        load_version_history(self.operations.store(), &self.project_id)
    }

    pub fn restore_checkpoint(
        &mut self,
        spec: &RestoreCheckpointSpec,
    ) -> Result<RestoreCheckpointResponse, WorkspaceCommandError> {
        restore_checkpoint(
            self.operations.store_mut(),
            &self.project_id,
            &self.main_branch_id,
            spec,
        )
    }
}

#[derive(Debug)]
pub enum ProjectPackageError {
    Validation(String),
    AlreadyExists,
    InvalidPackage(&'static str),
    Host(HostError),
    Io {
        operation: &'static str,
        source: std::io::Error,
    },
    ManifestJson(serde_json::Error),
    Store(StoreError),
    Operation(crate::OperationCommandError),
    Workspace(WorkspaceCommandError),
    Clock,
}

impl ProjectPackageError {
    pub fn code(&self) -> &'static str {
        match self {
            Self::Validation(_) => "INVALID_PROJECT_REQUEST",
            Self::AlreadyExists => "PROJECT_ALREADY_EXISTS",
            Self::InvalidPackage(_) | Self::ManifestJson(_) => "INVALID_PROJECT_PACKAGE",
            Self::Host(_) => "UNSAFE_PROJECT_PATH",
            Self::Io { .. } => "PROJECT_IO_FAILED",
            Self::Store(_) | Self::Operation(_) | Self::Workspace(_) => "PROJECT_STORAGE_FAILED",
            Self::Clock => "HOST_CLOCK_FAILED",
        }
    }

    pub fn public_message(&self) -> String {
        match self {
            Self::Validation(message) => message.clone(),
            Self::AlreadyExists => "Project package already exists".into(),
            Self::InvalidPackage(reason) => format!("Invalid project package: {reason}"),
            Self::Host(_) => "Project path is invalid or unavailable".into(),
            Self::Io { operation, .. } => format!("Project file operation failed: {operation}"),
            Self::ManifestJson(_) => "Project manifest is invalid".into(),
            Self::Store(_) | Self::Operation(_) | Self::Workspace(_) => {
                "Project storage operation failed".into()
            }
            Self::Clock => "System clock could not create a project timestamp".into(),
        }
    }
}

impl fmt::Display for ProjectPackageError {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        formatter.write_str(&self.public_message())
    }
}

impl std::error::Error for ProjectPackageError {
    fn source(&self) -> Option<&(dyn std::error::Error + 'static)> {
        match self {
            Self::Host(error) => Some(error),
            Self::Io { source, .. } => Some(source),
            Self::ManifestJson(error) => Some(error),
            Self::Store(error) => Some(error),
            Self::Operation(error) => Some(error),
            Self::Workspace(error) => Some(error),
            Self::Validation(_) | Self::AlreadyExists | Self::InvalidPackage(_) | Self::Clock => {
                None
            }
        }
    }
}

fn create_staged_package(path: &Path, spec: &NewProjectSpec) -> Result<(), ProjectPackageError> {
    for directory in ["assets", "backups", "exports"] {
        fs::create_dir(path.join(directory)).map_err(|source| ProjectPackageError::Io {
            operation: "create project subdirectory",
            source,
        })?;
    }

    let created_at = OffsetDateTime::now_utc()
        .format(&Rfc3339)
        .map_err(|_| ProjectPackageError::Clock)?;
    let project_id = id("project");
    let document_id = id("document");
    let block_id = id("block");
    let commit_id = id("commit");
    let branch_id = id("branch");
    let content = serde_json::json!({ "type": "paragraph", "text": "" });
    let content_json =
        serde_json::to_string(&content).map_err(ProjectPackageError::ManifestJson)?;
    let content_hash = block_content_hash("paragraph", &content, "", false)
        .map_err(ProjectPackageError::Workspace)?;
    let root_hash = project_root_hash([(block_id.as_str(), content_hash.as_str())]);
    let seed = ProjectSeed {
        project_id: project_id.clone(),
        title: spec.title.trim().into(),
        language: spec.language.clone(),
        initial_commit_id: commit_id,
        initial_root_hash: root_hash,
        main_branch_id: branch_id.clone(),
        documents: vec![SeedDocument {
            id: document_id.clone(),
            parent_id: None,
            kind: "chapter".into(),
            title: "正文".into(),
            order_key: "a0".into(),
        }],
        blocks: vec![SeedBlock {
            id: block_id,
            document_id,
            kind: "paragraph".into(),
            order_key: "a0".into(),
            content_json,
            plain_text: String::new(),
            content_hash,
            locked: false,
        }],
        created_at: created_at.clone(),
    };
    let database_path = path.join(DATABASE_FILE);
    {
        let mut store = OptimizerStore::open(&database_path).map_err(ProjectPackageError::Store)?;
        store
            .initialize_project(&seed)
            .map_err(ProjectPackageError::Store)?;
        store
            .verify_invariants()
            .map_err(ProjectPackageError::Store)?;
    }

    let manifest = ProjectPackageManifest {
        schema_version: PACKAGE_SCHEMA_VERSION,
        project_id,
        main_branch_id: branch_id,
        title: spec.title.trim().into(),
        language: spec.language.clone(),
        database: DATABASE_FILE.into(),
        created_at,
    };
    let manifest_bytes =
        serde_json::to_vec_pretty(&manifest).map_err(ProjectPackageError::ManifestJson)?;
    let temporary_manifest = path.join("manifest.json.tmp");
    fs::write(&temporary_manifest, manifest_bytes).map_err(|source| ProjectPackageError::Io {
        operation: "write project manifest",
        source,
    })?;
    fs::rename(temporary_manifest, path.join(MANIFEST_FILE)).map_err(|source| {
        ProjectPackageError::Io {
            operation: "publish project manifest",
            source,
        }
    })?;
    Ok(())
}

fn validate_spec(spec: &NewProjectSpec) -> Result<(), ProjectPackageError> {
    let title = spec.title.trim();
    if title.is_empty() || title.chars().count() > 200 || title.chars().any(char::is_control) {
        return Err(ProjectPackageError::Validation(
            "Project title must contain 1..=200 printable characters".into(),
        ));
    }
    if spec.language.len() < 2
        || spec.language.len() > 35
        || !spec
            .language
            .bytes()
            .all(|byte| byte.is_ascii_alphanumeric() || byte == b'-')
    {
        return Err(ProjectPackageError::Validation(
            "Project language must be a valid language tag".into(),
        ));
    }
    let folder = Path::new(&spec.folder_name);
    if spec.folder_name.len() > 128
        || folder.components().count() != 1
        || !matches!(folder.components().next(), Some(Component::Normal(_)))
        || !spec
            .folder_name
            .to_ascii_lowercase()
            .ends_with(".optimizer")
        || spec.folder_name.chars().any(|character| {
            character.is_control() || matches!(character, '<' | '>' | ':' | '"' | '|' | '?' | '*')
        })
        || is_reserved_windows_name(&spec.folder_name)
    {
        return Err(ProjectPackageError::Validation(
            "Project folder must be one safe name ending in .optimizer".into(),
        ));
    }
    Ok(())
}

fn validate_manifest(manifest: &ProjectPackageManifest) -> Result<(), ProjectPackageError> {
    if manifest.schema_version != PACKAGE_SCHEMA_VERSION {
        return Err(ProjectPackageError::InvalidPackage(
            "unsupported manifest schema",
        ));
    }
    if manifest.database != DATABASE_FILE
        || manifest.project_id.trim().is_empty()
        || manifest.main_branch_id.trim().is_empty()
        || manifest.title.trim().is_empty()
        || manifest.language.trim().is_empty()
        || manifest.created_at.trim().is_empty()
    {
        return Err(ProjectPackageError::InvalidPackage(
            "manifest fields are invalid",
        ));
    }
    Ok(())
}

fn is_reserved_windows_name(folder_name: &str) -> bool {
    let stem = folder_name
        .split('.')
        .next()
        .unwrap_or_default()
        .to_ascii_uppercase();
    matches!(stem.as_str(), "CON" | "PRN" | "AUX" | "NUL")
        || stem
            .strip_prefix("COM")
            .or_else(|| stem.strip_prefix("LPT"))
            .is_some_and(|suffix| {
                matches!(suffix, "1" | "2" | "3" | "4" | "5" | "6" | "7" | "8" | "9")
            })
}

fn cleanup_staging_directory(parent: &ProjectRoot, staging_path: &Path) {
    if staging_path.parent() == Some(parent.as_path())
        && staging_path
            .file_name()
            .and_then(|name| name.to_str())
            .is_some_and(|name| name.contains(".creating-"))
    {
        let _ = fs::remove_dir_all(staging_path);
    }
}

fn id(prefix: &str) -> String {
    format!("{prefix}-{}", Uuid::new_v4().simple())
}

fn project_info(
    root: &ProjectRoot,
    main_branch_id: &str,
    database_schema_version: i64,
    record: ProjectRecord,
) -> ProjectInfo {
    ProjectInfo {
        schema_version: PACKAGE_SCHEMA_VERSION,
        project_id: record.id,
        main_branch_id: main_branch_id.into(),
        title: record.title,
        language: record.language,
        directory: root.as_path().to_string_lossy().into_owned(),
        database_schema_version,
        head_commit_id: record.head_commit_id,
        revision: record.revision,
        created_at: record.created_at,
        updated_at: record.updated_at,
    }
}

#[cfg(test)]
mod tests {
    use std::path::PathBuf;
    use std::sync::atomic::{AtomicU64, Ordering};
    use std::time::{SystemTime, UNIX_EPOCH};

    use sha2::{Digest, Sha256};

    use super::*;

    struct TempParent(PathBuf);

    static NEXT_TEMP_DIRECTORY: AtomicU64 = AtomicU64::new(0);

    impl TempParent {
        fn new() -> Self {
            let nonce = SystemTime::now()
                .duration_since(UNIX_EPOCH)
                .unwrap()
                .as_nanos();
            let path = std::env::temp_dir().join(format!(
                "optimizer-project-package-{}-{nonce}-{}",
                std::process::id(),
                NEXT_TEMP_DIRECTORY.fetch_add(1, Ordering::Relaxed)
            ));
            fs::create_dir_all(&path).unwrap();
            Self(path)
        }
    }

    impl Drop for TempParent {
        fn drop(&mut self) {
            let _ = fs::remove_dir_all(&self.0);
        }
    }

    fn spec() -> NewProjectSpec {
        NewProjectSpec {
            folder_name: "MyNovel.optimizer".into(),
            title: "My Novel".into(),
            language: "zh-CN".into(),
        }
    }

    #[test]
    fn creates_an_atomic_project_package_and_reopens_it() {
        let parent = TempParent::new();
        let project = OpenedProject::create(&parent.0, &spec()).unwrap();
        let info = project.info().unwrap();
        assert_eq!(info.title, "My Novel");
        assert_eq!(info.language, "zh-CN");
        assert!(info.main_branch_id.starts_with("branch-"));
        assert!(Path::new(&info.directory).join(DATABASE_FILE).is_file());
        assert!(Path::new(&info.directory).join(MANIFEST_FILE).is_file());
        for directory in ["assets", "backups", "exports"] {
            assert!(Path::new(&info.directory).join(directory).is_dir());
        }
        let project_id = info.project_id;
        drop(project);

        let reopened = OpenedProject::open(parent.0.join("MyNovel.optimizer")).unwrap();
        assert_eq!(reopened.info().unwrap().project_id, project_id);
    }

    #[test]
    fn loads_the_workspace_and_saves_a_block_as_a_versioned_commit() {
        let parent = TempParent::new();
        let mut project = OpenedProject::create(&parent.0, &spec()).unwrap();
        let initial = project.workspace().unwrap();
        assert_eq!(initial.documents.len(), 1);
        assert_eq!(initial.blocks.len(), 1);
        let block = initial.blocks[0].clone();
        let saved = project
            .save_block(&SaveBlockSpec {
                block_id: block.id.clone(),
                expected_revision: block.revision,
                expected_hash: block.content_hash.clone(),
                content: serde_json::json!({
                    "type": "paragraph",
                    "content": [{ "type": "text", "text": "第一段正文" }]
                }),
                plain_text: "第一段正文".into(),
            })
            .unwrap();
        assert_eq!(saved.project_revision, 1);
        assert_eq!(saved.block.revision, 1);
        assert_eq!(saved.block.plain_text, "第一段正文");
        assert_eq!(saved.head_commit_id, saved.commit_id);
        assert_ne!(saved.previous_head_commit_id, saved.commit_id);
        assert!(saved.block.content_hash.starts_with("sha256:"));

        let reloaded = project.workspace().unwrap();
        assert_eq!(reloaded.revision, 1);
        assert_eq!(reloaded.documents[0].revision, 1);
        assert_eq!(reloaded.head_commit_id, saved.commit_id);
        assert_eq!(reloaded.blocks[0], saved.block);
        assert!(matches!(
            project.save_block(&SaveBlockSpec {
                block_id: saved.block.id.clone(),
                expected_revision: saved.block.revision,
                expected_hash: saved.block.content_hash.clone(),
                content: saved.block.content.clone(),
                plain_text: saved.block.plain_text.clone(),
            }),
            Err(WorkspaceCommandError::NoChanges)
        ));
        assert!(matches!(
            project.save_block(&SaveBlockSpec {
                block_id: block.id,
                expected_revision: block.revision,
                expected_hash: block.content_hash,
                content: serde_json::json!({ "type": "paragraph", "text": "stale" }),
                plain_text: "stale".into(),
            }),
            Err(WorkspaceCommandError::Store(StoreError::Conflict { .. }))
        ));
    }

    #[test]
    fn atomically_applies_a_reviewed_proposal_as_an_ai_accept_commit() {
        let parent = TempParent::new();
        let mut project = OpenedProject::create(&parent.0, &spec()).unwrap();
        let workspace = project.workspace().unwrap();
        let block = &workspace.blocks[0];
        let run_id = "run-apply-test";
        let intent_id = "intent-apply-test";
        let proposal_id = "proposal-apply-test";
        let created_at = "2026-07-15T00:00:00Z";
        let mut proposal = serde_json::json!({
            "schemaVersion": 2,
            "id": proposal_id,
            "operationRunId": run_id,
            "baseCommitId": workspace.head_commit_id,
            "target": {
                "documentId": block.document_id,
                "blockId": block.id,
                "baseRevision": block.revision,
                "baseHash": block.content_hash,
                "from": { "blockId": block.id, "offset": 0 },
                "to": { "blockId": block.id, "offset": 0 }
            },
            "hunks": [{
                "id": "proposal-apply-test:h1",
                "from": { "blockId": block.id, "offset": 0, "affinity": "after" },
                "to": { "blockId": block.id, "offset": 0, "affinity": "before" },
                "original": "",
                "replacement": "你好，世界",
                "granularity": "token"
            }],
            "warnings": [],
            "status": "review",
            "createdAt": created_at
        });
        let proposal_hash = format!(
            "sha256:{:x}",
            Sha256::digest(serde_json::to_vec(&proposal).unwrap())
        );
        proposal["proposalHash"] = serde_json::json!(proposal_hash);
        let transitions = [
            ("draft", "compiling"),
            ("compiling", "preflight"),
            ("preflight", "queued"),
            ("queued", "streaming"),
            ("streaming", "validating"),
            ("validating", "review"),
        ]
        .map(|(from, to)| {
            serde_json::json!({
                "fromState": from,
                "toState": to,
                "occurredAt": created_at
            })
        });
        let bundle = serde_json::json!({
            "schemaVersion": 1,
            "run": {
                "id": run_id,
                "operationIntentId": intent_id,
                "projectId": workspace.project_id,
                "baseCommitId": workspace.head_commit_id,
                "providerId": "deepseek",
                "model": "deepseek-v4-flash",
                "state": "review",
                "responseId": "response-apply-test",
                "finishReason": "stop",
                "startedAt": created_at,
                "updatedAt": created_at
            },
            "contextPacket": {
                "id": "context-apply-test",
                "operationIntentId": intent_id,
                "projectId": workspace.project_id,
                "baseCommitId": workspace.head_commit_id,
                "packetHash": "sha256:context-apply-test",
                "payload": { "schemaVersion": 1, "kind": "test" },
                "createdAt": created_at
            },
            "lifecycleEvents": transitions,
            "artifact": {
                "id": proposal_id,
                "kind": "patch_proposal",
                "bindingHash": proposal_hash,
                "payload": proposal,
                "createdAt": created_at
            }
        });
        project
            .operations_mut()
            .persist_operation_bundle_json(&bundle.to_string())
            .unwrap();
        project
            .operations_mut()
            .append_review_event_json(
                &serde_json::json!({
                    "schemaVersion": 1,
                    "id": "review-decision-apply-test",
                    "proposalId": proposal_id,
                    "expectedRevision": 0,
                    "expectedStatus": "review",
                    "kind": "decision",
                    "nextStatus": "ready",
                    "hunkId": "proposal-apply-test:h1",
                    "decision": "accepted",
                    "occurredAt": created_at
                })
                .to_string(),
            )
            .unwrap();

        let applied = project
            .apply_reviewed_proposal(&ApplyReviewedProposalSpec {
                proposal_id: proposal_id.into(),
                expected_review_revision: 1,
            })
            .unwrap();
        assert_eq!(applied.accepted_hunks, 1);
        assert_eq!(applied.rejected_hunks, 0);
        assert_eq!(applied.review_revision, 2);
        assert_eq!(applied.save.block.plain_text, "你好，世界");
        let commit = project
            .version_history()
            .unwrap()
            .commits
            .into_iter()
            .find(|commit| commit.id == applied.save.commit_id)
            .unwrap();
        assert_eq!(commit.reason, "ai_accept");
        assert_eq!(commit.actor_type, "model");
        assert_eq!(commit.actor_id.as_deref(), Some(run_id));
        let audit = project.operations().get_operation_audit(run_id).unwrap();
        assert_eq!(audit.run.state, "accepted");
        assert_eq!(audit.review.unwrap().status, "applied");
    }

    #[test]
    fn rejects_unsafe_or_existing_package_names_without_staging_debris() {
        let parent = TempParent::new();
        for folder_name in [
            "../escape.optimizer",
            "CON.optimizer",
            "bad?.optimizer",
            "plain",
        ] {
            let mut invalid = spec();
            invalid.folder_name = folder_name.into();
            assert!(matches!(
                OpenedProject::create(&parent.0, &invalid),
                Err(ProjectPackageError::Validation(_))
            ));
        }
        let project = OpenedProject::create(&parent.0, &spec()).unwrap();
        drop(project);
        assert!(matches!(
            OpenedProject::create(&parent.0, &spec()),
            Err(ProjectPackageError::AlreadyExists)
        ));
        assert_eq!(
            fs::read_dir(&parent.0)
                .unwrap()
                .filter_map(Result::ok)
                .filter(|entry| entry.file_name().to_string_lossy().contains(".creating-"))
                .count(),
            0
        );
    }

    #[test]
    fn rejects_manifest_tampering_before_exposing_the_store() {
        let parent = TempParent::new();
        let project = OpenedProject::create(&parent.0, &spec()).unwrap();
        let directory = project.info().unwrap().directory;
        drop(project);
        let manifest_path = Path::new(&directory).join(MANIFEST_FILE);
        let mut manifest: serde_json::Value =
            serde_json::from_slice(&fs::read(&manifest_path).unwrap()).unwrap();
        manifest["projectId"] = serde_json::Value::String("project-tampered".into());
        fs::write(&manifest_path, serde_json::to_vec(&manifest).unwrap()).unwrap();
        assert!(matches!(
            OpenedProject::open(directory),
            Err(ProjectPackageError::Store(StoreError::NotFound { .. }))
        ));
    }

    #[test]
    fn rejects_a_main_branch_manifest_that_does_not_bind_to_the_project() {
        let parent = TempParent::new();
        let project = OpenedProject::create(&parent.0, &spec()).unwrap();
        let directory = project.info().unwrap().directory;
        drop(project);
        let manifest_path = Path::new(&directory).join(MANIFEST_FILE);
        let mut manifest: serde_json::Value =
            serde_json::from_slice(&fs::read(&manifest_path).unwrap()).unwrap();
        manifest["mainBranchId"] = serde_json::Value::String("branch-tampered".into());
        fs::write(&manifest_path, serde_json::to_vec(&manifest).unwrap()).unwrap();
        assert!(matches!(
            OpenedProject::open(directory),
            Err(ProjectPackageError::Store(StoreError::NotFound { .. }))
        ));
    }
}
