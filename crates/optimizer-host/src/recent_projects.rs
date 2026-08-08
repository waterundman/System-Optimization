use std::collections::HashSet;
use std::fmt;
use std::fs::{self, OpenOptions};
use std::path::{Path, PathBuf};

use serde::{Deserialize, Serialize};
use time::OffsetDateTime;
use time::format_description::well_known::Rfc3339;
use uuid::Uuid;

use crate::ProjectInfo;

const REGISTRY_SCHEMA_VERSION: u32 = 1;
const RECENT_PROJECT_SCHEMA_VERSION: u32 = 1;
const MAX_RECENT_PROJECTS: usize = 12;
const MAX_REGISTRY_BYTES: u64 = 256 * 1024;

#[derive(Debug, Clone, PartialEq, Eq, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct RecentProject {
    pub schema_version: u32,
    pub project_id: String,
    pub title: String,
    pub language: String,
    pub directory: String,
    pub last_opened_at: String,
    pub available: bool,
    /// v0.8.0 Stage 1: `Some("backup")` when the project was restored from
    /// a `.optimizer-backup` archive. `None` for normally created/opened
    /// projects. The frontend uses this to badge restored projects.
    #[serde(skip_serializing_if = "Option::is_none")]
    pub source: Option<String>,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
struct RecentProjectEntry {
    schema_version: u32,
    project_id: String,
    title: String,
    language: String,
    directory: String,
    last_opened_at: String,
    /// v0.8.0 Stage 1: `Some("backup")` for restored projects.
    /// `#[serde(default)]` ensures backward compatibility with existing
    /// registry files that predate the source field.
    #[serde(default)]
    source: Option<String>,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
struct RecentProjectFile {
    schema_version: u32,
    projects: Vec<RecentProjectEntry>,
}

#[derive(Debug)]
pub enum RecentProjectError {
    PathMustBeAbsolute(PathBuf),
    InvalidRegistry(&'static str),
    Io {
        operation: &'static str,
        source: std::io::Error,
    },
    Json(serde_json::Error),
    Time(time::error::Format),
    NotFound,
}

impl RecentProjectError {
    pub fn code(&self) -> &'static str {
        match self {
            Self::NotFound => "RECENT_PROJECT_NOT_FOUND",
            Self::PathMustBeAbsolute(_)
            | Self::InvalidRegistry(_)
            | Self::Io { .. }
            | Self::Json(_)
            | Self::Time(_) => "RECENT_PROJECTS_UNAVAILABLE",
        }
    }

    pub fn public_message(&self) -> &'static str {
        match self {
            Self::NotFound => "The recent project is no longer registered",
            _ => "Recent projects are unavailable",
        }
    }
}

impl fmt::Display for RecentProjectError {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            Self::PathMustBeAbsolute(path) => {
                write!(
                    formatter,
                    "recent project registry path must be absolute: {}",
                    path.display()
                )
            }
            Self::InvalidRegistry(message) => {
                write!(formatter, "invalid recent project registry: {message}")
            }
            Self::Io { operation, source } => {
                write!(
                    formatter,
                    "failed to {operation} recent project registry: {source}"
                )
            }
            Self::Json(error) => write!(formatter, "invalid recent project registry JSON: {error}"),
            Self::Time(error) => write!(formatter, "failed to format recent project time: {error}"),
            Self::NotFound => write!(formatter, "recent project is not registered"),
        }
    }
}

impl std::error::Error for RecentProjectError {
    fn source(&self) -> Option<&(dyn std::error::Error + 'static)> {
        match self {
            Self::Io { source, .. } => Some(source),
            Self::Json(error) => Some(error),
            Self::Time(error) => Some(error),
            Self::PathMustBeAbsolute(_) | Self::InvalidRegistry(_) | Self::NotFound => None,
        }
    }
}

#[derive(Debug, Clone)]
pub struct RecentProjectRegistry {
    path: Option<PathBuf>,
    projects: Vec<RecentProjectEntry>,
}

impl Default for RecentProjectRegistry {
    fn default() -> Self {
        Self::memory()
    }
}

impl RecentProjectRegistry {
    pub fn memory() -> Self {
        Self {
            path: None,
            projects: Vec::new(),
        }
    }

    pub fn open(path: impl AsRef<Path>) -> Result<Self, RecentProjectError> {
        let path = path.as_ref();
        if !path.is_absolute() {
            return Err(RecentProjectError::PathMustBeAbsolute(path.to_path_buf()));
        }
        let parent = path.parent().ok_or(RecentProjectError::InvalidRegistry(
            "registry has no parent directory",
        ))?;
        fs::create_dir_all(parent).map_err(|source| RecentProjectError::Io {
            operation: "create parent directory for",
            source,
        })?;
        let projects = load_registry(path)?;
        validate_entries(&projects)?;
        Ok(Self {
            path: Some(path.to_path_buf()),
            projects,
        })
    }

    pub fn list(&self) -> Vec<RecentProject> {
        self.projects
            .iter()
            .map(|entry| RecentProject {
                schema_version: RECENT_PROJECT_SCHEMA_VERSION,
                project_id: entry.project_id.clone(),
                title: entry.title.clone(),
                language: entry.language.clone(),
                directory: entry.directory.clone(),
                last_opened_at: entry.last_opened_at.clone(),
                available: project_package_exists(&entry.directory),
                source: entry.source.clone(),
            })
            .collect()
    }

    pub fn directory_for(&self, project_id: &str) -> Result<String, RecentProjectError> {
        self.projects
            .iter()
            .find(|entry| entry.project_id == project_id)
            .map(|entry| entry.directory.clone())
            .ok_or(RecentProjectError::NotFound)
    }

    pub fn record(&mut self, info: &ProjectInfo) -> Result<(), RecentProjectError> {
        self.record_with_source(info, None)
    }

    /// v0.8.0 Stage 1: record a project with an optional source tag.
    /// When `source` is `Some("backup")`, the frontend can badge the
    /// project as restored from a `.optimizer-backup` archive.
    pub fn record_with_source(
        &mut self,
        info: &ProjectInfo,
        source: Option<&str>,
    ) -> Result<(), RecentProjectError> {
        let entry = RecentProjectEntry {
            schema_version: RECENT_PROJECT_SCHEMA_VERSION,
            project_id: info.project_id.clone(),
            title: info.title.clone(),
            language: info.language.clone(),
            directory: info.directory.clone(),
            last_opened_at: OffsetDateTime::now_utc()
                .format(&Rfc3339)
                .map_err(RecentProjectError::Time)?,
            source: source.map(str::to_owned),
        };
        validate_entry(&entry)?;
        let previous = self.projects.clone();
        self.projects.retain(|candidate| {
            candidate.project_id != entry.project_id && candidate.directory != entry.directory
        });
        self.projects.insert(0, entry);
        self.projects.truncate(MAX_RECENT_PROJECTS);
        if let Err(error) = self.persist() {
            self.projects = previous;
            return Err(error);
        }
        Ok(())
    }

    pub fn remove(&mut self, project_id: &str) -> Result<bool, RecentProjectError> {
        let previous = self.projects.clone();
        self.projects.retain(|entry| entry.project_id != project_id);
        if previous.len() == self.projects.len() {
            return Ok(false);
        }
        if let Err(error) = self.persist() {
            self.projects = previous;
            return Err(error);
        }
        Ok(true)
    }

    fn persist(&self) -> Result<(), RecentProjectError> {
        let Some(path) = self.path.as_ref() else {
            return Ok(());
        };
        let payload = serde_json::to_vec_pretty(&RecentProjectFile {
            schema_version: REGISTRY_SCHEMA_VERSION,
            projects: self.projects.clone(),
        })
        .map_err(RecentProjectError::Json)?;
        if payload.len() as u64 > MAX_REGISTRY_BYTES {
            return Err(RecentProjectError::InvalidRegistry(
                "registry exceeds size limit",
            ));
        }
        let parent = path.parent().ok_or(RecentProjectError::InvalidRegistry(
            "registry has no parent directory",
        ))?;
        let temporary = parent.join(format!(".recent-projects-{}.tmp", Uuid::new_v4().simple()));
        if let Err(source) = fs::write(&temporary, payload) {
            let _ = fs::remove_file(&temporary);
            return Err(RecentProjectError::Io {
                operation: "write temporary",
                source,
            });
        }
        if let Err(source) = OpenOptions::new()
            .write(true)
            .open(&temporary)
            .and_then(|file| file.sync_all())
        {
            let _ = fs::remove_file(&temporary);
            return Err(RecentProjectError::Io {
                operation: "flush temporary",
                source,
            });
        }
        replace_registry(path, &temporary)
    }
}

fn load_registry(path: &Path) -> Result<Vec<RecentProjectEntry>, RecentProjectError> {
    let backup = backup_path(path);
    if path.exists() && !path.is_file() {
        return Err(RecentProjectError::InvalidRegistry(
            "registry path is not a file",
        ));
    }
    let source = if path.is_file() {
        path
    } else if backup.is_file() {
        backup.as_path()
    } else {
        return Ok(Vec::new());
    };
    let metadata = fs::metadata(source).map_err(|source| RecentProjectError::Io {
        operation: "inspect",
        source,
    })?;
    if metadata.len() == 0 || metadata.len() > MAX_REGISTRY_BYTES {
        return Err(RecentProjectError::InvalidRegistry(
            "registry size is invalid",
        ));
    }
    let payload = fs::read(source).map_err(|source| RecentProjectError::Io {
        operation: "read",
        source,
    })?;
    let file: RecentProjectFile =
        serde_json::from_slice(&payload).map_err(RecentProjectError::Json)?;
    if file.schema_version != REGISTRY_SCHEMA_VERSION {
        return Err(RecentProjectError::InvalidRegistry(
            "unsupported schema version",
        ));
    }
    Ok(file.projects)
}

fn validate_entries(entries: &[RecentProjectEntry]) -> Result<(), RecentProjectError> {
    if entries.len() > MAX_RECENT_PROJECTS {
        return Err(RecentProjectError::InvalidRegistry("too many entries"));
    }
    let mut project_ids = HashSet::new();
    let mut directories = HashSet::new();
    for entry in entries {
        validate_entry(entry)?;
        if !project_ids.insert(entry.project_id.as_str()) {
            return Err(RecentProjectError::InvalidRegistry("duplicate project id"));
        }
        if !directories.insert(entry.directory.as_str()) {
            return Err(RecentProjectError::InvalidRegistry(
                "duplicate project directory",
            ));
        }
    }
    Ok(())
}

fn validate_entry(entry: &RecentProjectEntry) -> Result<(), RecentProjectError> {
    if entry.schema_version != RECENT_PROJECT_SCHEMA_VERSION {
        return Err(RecentProjectError::InvalidRegistry(
            "unsupported entry schema",
        ));
    }
    if !is_safe_id(&entry.project_id) {
        return Err(RecentProjectError::InvalidRegistry("invalid project id"));
    }
    if entry.title.trim().is_empty() || entry.title.len() > 512 {
        return Err(RecentProjectError::InvalidRegistry("invalid project title"));
    }
    if entry.language.trim().is_empty() || entry.language.len() > 64 {
        return Err(RecentProjectError::InvalidRegistry(
            "invalid project language",
        ));
    }
    let directory = Path::new(&entry.directory);
    if !directory.is_absolute() || entry.directory.len() > 32_767 || entry.directory.contains('\0')
    {
        return Err(RecentProjectError::InvalidRegistry(
            "invalid project directory",
        ));
    }
    if OffsetDateTime::parse(&entry.last_opened_at, &Rfc3339).is_err() {
        return Err(RecentProjectError::InvalidRegistry(
            "invalid last-opened time",
        ));
    }
    Ok(())
}

fn is_safe_id(value: &str) -> bool {
    !value.is_empty()
        && value.len() <= 128
        && value
            .bytes()
            .all(|byte| byte.is_ascii_alphanumeric() || matches!(byte, b'_' | b'-'))
}

fn project_package_exists(directory: &str) -> bool {
    let directory = Path::new(directory);
    directory.is_dir() && directory.join("manifest.json").is_file()
}

fn backup_path(path: &Path) -> PathBuf {
    let file_name = path
        .file_name()
        .and_then(|name| name.to_str())
        .unwrap_or("recent-projects.json");
    path.with_file_name(format!("{file_name}.bak"))
}

fn replace_registry(path: &Path, temporary: &Path) -> Result<(), RecentProjectError> {
    let backup = backup_path(path);
    if backup.exists()
        && let Err(source) = fs::remove_file(&backup)
    {
        let _ = fs::remove_file(temporary);
        return Err(RecentProjectError::Io {
            operation: "remove stale backup for",
            source,
        });
    }
    if path.exists() {
        fs::rename(path, &backup).map_err(|source| RecentProjectError::Io {
            operation: "stage previous",
            source,
        })?;
    }
    if let Err(source) = fs::rename(temporary, path) {
        if backup.exists() {
            let _ = fs::rename(&backup, path);
        }
        let _ = fs::remove_file(temporary);
        return Err(RecentProjectError::Io {
            operation: "publish",
            source,
        });
    }
    if backup.exists() {
        let _ = fs::remove_file(&backup);
    }
    Ok(())
}

#[cfg(test)]
mod tests {
    use std::fs;
    use std::sync::atomic::{AtomicU64, Ordering};
    use std::time::{SystemTime, UNIX_EPOCH};

    use super::*;

    static NEXT_TEMP: AtomicU64 = AtomicU64::new(0);

    struct TempDirectory(PathBuf);

    impl TempDirectory {
        fn new() -> Self {
            let nonce = SystemTime::now()
                .duration_since(UNIX_EPOCH)
                .unwrap()
                .as_nanos();
            let path = std::env::temp_dir().join(format!(
                "optimizer-recent-projects-{}-{nonce}-{}",
                std::process::id(),
                NEXT_TEMP.fetch_add(1, Ordering::Relaxed)
            ));
            fs::create_dir_all(&path).unwrap();
            Self(path)
        }
    }

    impl Drop for TempDirectory {
        fn drop(&mut self) {
            let _ = fs::remove_dir_all(&self.0);
        }
    }

    fn info(project_id: &str, directory: &Path, title: &str) -> ProjectInfo {
        ProjectInfo {
            schema_version: 1,
            project_id: project_id.into(),
            main_branch_id: format!("branch-{project_id}"),
            title: title.into(),
            language: "zh-CN".into(),
            directory: directory.to_string_lossy().into_owned(),
            database_schema_version: 5,
            head_commit_id: format!("commit-{project_id}"),
            revision: 0,
            created_at: "2026-07-16T00:00:00Z".into(),
            updated_at: "2026-07-16T00:00:00Z".into(),
        }
    }

    #[test]
    fn persists_deduplicates_and_reloads_recent_projects() {
        let temp = TempDirectory::new();
        let first = temp.0.join("first.optimizer");
        let second = temp.0.join("second.optimizer");
        for directory in [&first, &second] {
            fs::create_dir(directory).unwrap();
            fs::write(directory.join("manifest.json"), b"{}").unwrap();
        }
        let registry_path = temp.0.join("recent-projects.json");
        let mut registry = RecentProjectRegistry::open(&registry_path).unwrap();
        registry
            .record(&info("project-1", &first, "First"))
            .unwrap();
        registry
            .record(&info("project-2", &second, "Second"))
            .unwrap();
        registry
            .record(&info("project-1", &first, "First renamed"))
            .unwrap();

        let reloaded = RecentProjectRegistry::open(&registry_path).unwrap();
        let projects = reloaded.list();
        assert_eq!(projects.len(), 2);
        assert_eq!(projects[0].project_id, "project-1");
        assert_eq!(projects[0].title, "First renamed");
        assert!(projects.iter().all(|project| project.available));
        assert_eq!(
            reloaded.directory_for("project-2").unwrap(),
            second.to_string_lossy()
        );
    }

    #[test]
    fn removal_is_persistent_and_missing_packages_are_marked_unavailable() {
        let temp = TempDirectory::new();
        let package = temp.0.join("missing.optimizer");
        fs::create_dir(&package).unwrap();
        fs::write(package.join("manifest.json"), b"{}").unwrap();
        let registry_path = temp.0.join("recent-projects.json");
        let mut registry = RecentProjectRegistry::open(&registry_path).unwrap();
        registry
            .record(&info("project-1", &package, "Missing"))
            .unwrap();
        fs::remove_dir_all(&package).unwrap();
        assert!(!registry.list()[0].available);
        assert!(registry.remove("project-1").unwrap());
        assert!(!registry.remove("project-1").unwrap());
        assert!(
            RecentProjectRegistry::open(registry_path)
                .unwrap()
                .list()
                .is_empty()
        );
    }

    #[test]
    fn keeps_only_the_twelve_most_recent_unique_projects() {
        let temp = TempDirectory::new();
        let mut registry = RecentProjectRegistry::memory();
        for index in 0..13 {
            registry
                .record(&info(
                    &format!("project-{index}"),
                    &temp.0.join(format!("project-{index}.optimizer")),
                    &format!("Project {index}"),
                ))
                .unwrap();
        }
        let projects = registry.list();
        assert_eq!(projects.len(), MAX_RECENT_PROJECTS);
        assert_eq!(projects[0].project_id, "project-12");
        assert!(
            projects
                .iter()
                .all(|project| project.project_id != "project-0")
        );
    }

    #[test]
    fn rejects_tampered_or_oversized_registries() {
        let temp = TempDirectory::new();
        let path = temp.0.join("recent-projects.json");
        fs::write(&path, br#"{"schemaVersion":1,"projects":[],"extra":true}"#).unwrap();
        assert!(matches!(
            RecentProjectRegistry::open(&path),
            Err(RecentProjectError::Json(_))
        ));
        fs::write(&path, vec![b'x'; MAX_REGISTRY_BYTES as usize + 1]).unwrap();
        assert!(matches!(
            RecentProjectRegistry::open(path),
            Err(RecentProjectError::InvalidRegistry(_))
        ));
    }
}
