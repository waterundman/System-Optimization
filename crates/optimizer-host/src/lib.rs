use std::fmt;
use std::fs;
use std::path::{Component, Path, PathBuf};

mod model_gateway;
mod model_transport;
mod operation_commands;
mod operation_context;
mod project_package;
mod recent_projects;
mod secrets;
mod summary_worker;
mod workspace_commands;

pub use model_gateway::{
    CancelModelRequestResponse, ModelAuthorizationScope, ModelExecutionHost, ModelExecutionRequest,
    ModelExecutionSummary, ModelGatewayError, ModelProviderConfiguration, ModelProviderId,
    ModelRequestAuthorization, ModelStreamEvent, ModelTransport, NativeModelTransport,
    OllamaModelInfo, OllamaModelList,
};
pub use operation_commands::{
    ArtifactAudit, ContextPacketAudit, FailureAudit, LifecycleEventAudit, ModelUsageAudit,
    OperationAuditResponse, OperationCommandError, OperationCommandHost, PersistOperationResponse,
    PersistReviewResponse, ReviewAudit, ReviewEventAudit, RunAudit,
};
pub use operation_context::{
    OperationContextCandidate, OperationContextSignals, OperationContextSpec,
};
pub use project_package::{
    ExportMarkdownResponse, NewProjectSpec, OpenedProject, ProjectInfo, ProjectPackageError,
    ProjectPackageManifest,
};
pub use recent_projects::{RecentProject, RecentProjectError, RecentProjectRegistry};
#[cfg(windows)]
pub use secrets::WindowsCredentialStore;
pub use secrets::{MemorySecretStore, SecretReference, SecretStore, SecretStoreError, SecretValue};
pub use summary_worker::{
    RefreshSummariesSpec, SummaryContextCandidate, SummaryContextSpec, SummaryRefreshReport,
};
pub use workspace_commands::{
    ApplyReviewedProposalResponse, ApplyReviewedProposalSpec, ArchivedDocument,
    ChangeDocumentDepthSpec, CheckpointSummary, CreateDocumentResponse, CreateDocumentSpec,
    CreateKnowledgeItemSpec, CreateStyleSampleSpec, DocumentDepthDirection, DocumentMoveDirection,
    DocumentMutationResponse, KnowledgeContextCandidate, KnowledgeContextSpec, KnowledgeItem,
    ProjectWorkspace, RenameDocumentSpec, ReorderDocumentSpec, RestoreCheckpointResponse,
    RestoreCheckpointSpec, SaveBlockResponse, SaveBlockSpec, SetDocumentArchivedSpec,
    SetKnowledgeItemStatusSpec, SetStyleSampleStatusSpec, StyleSample, SummaryInvalidation,
    VersionCommit, VersionHistory, WorkspaceBlock, WorkspaceCommandError, WorkspaceDocument,
};

#[derive(Debug)]
pub enum HostError {
    RootMustBeAbsolute(PathBuf),
    RootUnavailable(std::io::Error),
    UnsafeRelativePath(PathBuf),
    TargetUnavailable(std::io::Error),
    PathOutsideProject { root: PathBuf, target: PathBuf },
}

impl fmt::Display for HostError {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            Self::RootMustBeAbsolute(path) => write!(
                formatter,
                "project root must be absolute: {}",
                path.display()
            ),
            Self::RootUnavailable(error) => {
                write!(formatter, "project root is unavailable: {error}")
            }
            Self::UnsafeRelativePath(path) => write!(
                formatter,
                "path is not a safe project-relative path: {}",
                path.display()
            ),
            Self::TargetUnavailable(error) => {
                write!(formatter, "target path is unavailable: {error}")
            }
            Self::PathOutsideProject { root, target } => write!(
                formatter,
                "resolved path {} is outside project root {}",
                target.display(),
                root.display()
            ),
        }
    }
}

impl std::error::Error for HostError {}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct ProjectRoot {
    canonical: PathBuf,
}

impl ProjectRoot {
    pub fn open(root: impl AsRef<Path>) -> Result<Self, HostError> {
        let root = root.as_ref();
        if !root.is_absolute() {
            return Err(HostError::RootMustBeAbsolute(root.to_path_buf()));
        }
        let canonical = fs::canonicalize(root).map_err(HostError::RootUnavailable)?;
        Ok(Self { canonical })
    }

    pub fn as_path(&self) -> &Path {
        &self.canonical
    }

    pub fn resolve_existing(&self, relative: impl AsRef<Path>) -> Result<PathBuf, HostError> {
        let relative = relative.as_ref();
        validate_relative(relative)?;
        let target = fs::canonicalize(self.canonical.join(relative))
            .map_err(HostError::TargetUnavailable)?;
        self.ensure_inside(target)
    }

    pub fn resolve_for_create(&self, relative: impl AsRef<Path>) -> Result<PathBuf, HostError> {
        let relative = relative.as_ref();
        validate_relative(relative)?;
        let joined = self.canonical.join(relative);
        let parent = joined
            .parent()
            .ok_or_else(|| HostError::UnsafeRelativePath(relative.to_path_buf()))?;
        let canonical_parent = fs::canonicalize(parent).map_err(HostError::TargetUnavailable)?;
        self.ensure_inside(canonical_parent)?;
        let file_name = joined
            .file_name()
            .ok_or_else(|| HostError::UnsafeRelativePath(relative.to_path_buf()))?;
        Ok(parent.join(file_name))
    }

    fn ensure_inside(&self, target: PathBuf) -> Result<PathBuf, HostError> {
        if target.starts_with(&self.canonical) {
            Ok(target)
        } else {
            Err(HostError::PathOutsideProject {
                root: self.canonical.clone(),
                target,
            })
        }
    }
}

fn validate_relative(path: &Path) -> Result<(), HostError> {
    if path.as_os_str().is_empty() || path.is_absolute() {
        return Err(HostError::UnsafeRelativePath(path.to_path_buf()));
    }
    for component in path.components() {
        match component {
            Component::Normal(_) | Component::CurDir => {}
            Component::ParentDir | Component::RootDir | Component::Prefix(_) => {
                return Err(HostError::UnsafeRelativePath(path.to_path_buf()));
            }
        }
    }
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::time::{SystemTime, UNIX_EPOCH};

    struct TempProject {
        path: PathBuf,
    }

    impl TempProject {
        fn new() -> Self {
            let nonce = SystemTime::now()
                .duration_since(UNIX_EPOCH)
                .unwrap()
                .as_nanos();
            let path =
                std::env::temp_dir().join(format!("optimizer-host-{}-{nonce}", std::process::id()));
            fs::create_dir_all(path.join("assets")).unwrap();
            fs::write(path.join("assets").join("note.txt"), b"safe").unwrap();
            Self { path }
        }
    }

    impl Drop for TempProject {
        fn drop(&mut self) {
            let _ = fs::remove_dir_all(&self.path);
        }
    }

    #[test]
    fn resolves_existing_files_inside_the_project() {
        let temp = TempProject::new();
        let root = ProjectRoot::open(&temp.path).unwrap();
        let resolved = root
            .resolve_existing(Path::new("assets").join("note.txt"))
            .unwrap();
        assert!(resolved.starts_with(root.as_path()));
    }

    #[test]
    fn rejects_parent_directory_traversal() {
        let temp = TempProject::new();
        let root = ProjectRoot::open(&temp.path).unwrap();
        assert!(matches!(
            root.resolve_existing(Path::new("..").join("secret.txt")),
            Err(HostError::UnsafeRelativePath(_))
        ));
    }

    #[test]
    fn resolves_new_files_only_beneath_an_existing_safe_parent() {
        let temp = TempProject::new();
        let root = ProjectRoot::open(&temp.path).unwrap();
        let resolved = root
            .resolve_for_create(Path::new("assets").join("new.txt"))
            .unwrap();
        assert_eq!(resolved.parent().unwrap(), root.as_path().join("assets"));
    }
}
