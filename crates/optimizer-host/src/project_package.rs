use std::fmt;
use std::fmt::Write as _;
use std::fs;
use std::path::{Component, Path};

use optimizer_store::{
    OptimizerStore, ProjectRecord, ProjectSeed, SeedBlock, SeedDocument, StoreError,
};
use serde::{Deserialize, Serialize};
use sha2::{Digest, Sha256};
use time::OffsetDateTime;
use time::format_description::well_known::Rfc3339;
use uuid::Uuid;

use crate::{HostError, OperationCommandHost, ProjectRoot};

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

    pub fn operations(&self) -> &OperationCommandHost {
        &self.operations
    }

    pub fn operations_mut(&mut self) -> &mut OperationCommandHost {
        &mut self.operations
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
            Self::Store(_) | Self::Operation(_) => "PROJECT_STORAGE_FAILED",
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
            Self::Store(_) | Self::Operation(_) => "Project storage operation failed".into(),
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
    let content_json = r#"{"type":"paragraph","text":""}"#.to_string();
    let content_hash = sha256(&content_json);
    let root_hash = sha256(&format!("{block_id}:{content_hash}"));
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

fn sha256(value: &str) -> String {
    let digest = Sha256::digest(value.as_bytes());
    let mut encoded = String::with_capacity(71);
    encoded.push_str("sha256:");
    for byte in digest {
        let _ = write!(encoded, "{byte:02x}");
    }
    encoded
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
