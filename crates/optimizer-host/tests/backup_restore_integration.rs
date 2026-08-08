//! Integration tests for v0.8.0 Stage 1 (FR-11) project-level backup
//! and restore.
//!
//! These tests exercise the `export_project_backup` and
//! `import_project_backup` host functions end-to-end against real
//! `.optimizer` project packages on the file system.

use std::fs;
use std::io::{Read, Write};
use std::path::{Path, PathBuf};
use std::sync::atomic::{AtomicU64, Ordering};

use optimizer_host::{
    ExportProjectBackupRequest, ImportProjectBackupRequest, NewProjectSpec, OpenedProject,
};

/// RAII guard for a temporary directory. Removes the directory on drop.
struct TempDir {
    path: PathBuf,
}

impl TempDir {
    fn new(prefix: &str) -> Self {
        static COUNTER: AtomicU64 = AtomicU64::new(0);
        let id = COUNTER.fetch_add(1, Ordering::SeqCst);
        let pid = std::process::id();
        let dir = std::env::temp_dir().join(format!(
            "optimizer-test-{}-{}-{}",
            prefix, pid, id
        ));
        fs::create_dir_all(&dir).expect("failed to create temp dir");
        Self { path: dir }
    }

    fn path(&self) -> &Path {
        &self.path
    }

    fn join(&self, relative: &str) -> PathBuf {
        self.path.join(relative)
    }
}

impl Drop for TempDir {
    fn drop(&mut self) {
        let _ = fs::remove_dir_all(&self.path);
    }
}

/// Create a fresh `.optimizer` project package inside `parent`.
fn create_project(parent: &Path, folder_name: &str, title: &str) -> OpenedProject {
    let spec = NewProjectSpec {
        folder_name: folder_name.into(),
        title: title.into(),
        language: "zh-CN".into(),
    };
    OpenedProject::create(parent, &spec).expect("create project failed")
}

/// T01: export_project_backup produces a single .optimizer-backup zip file.
#[test]
fn t01_export_project_backup_creates_optimizer_backup_zip() {
    let temp = TempDir::new("t01");
    let project = create_project(temp.path(), "novel.optimizer", "Novel");
    let project_id = project.info().unwrap().project_id;
    let output_path = temp.join("novel.optimizer-backup");

    let response = project
        .export_project_backup(&ExportProjectBackupRequest {
            project_id: project_id.clone(),
            include_endpoints: false,
            include_recent: false,
            output_path: output_path.to_string_lossy().into_owned(),
            endpoints_json: None,
            recent_json: None,
        })
        .expect("export_project_backup failed");

    assert_eq!(response.schema_version, 1);
    assert!(
        response.archive_path.ends_with(".optimizer-backup"),
        "archive_path must end with .optimizer-backup, got: {}",
        response.archive_path
    );
    let metadata = fs::metadata(&response.archive_path)
        .expect("archive file must exist on disk");
    assert!(metadata.is_file(), "archive must be a single file");
    assert!(metadata.len() > 0, "archive must not be empty");
    assert_eq!(response.bytes_written, metadata.len());
    assert!(response.item_count > 0, "at least manifest.json + database");

    // Verify it is a valid zip archive.
    let file = fs::File::open(&response.archive_path).unwrap();
    let mut zip = zip::ZipArchive::new(file).expect("archive must be a valid zip");
    assert!(!zip.is_empty(), "zip must contain at least one entry");

    // Verify manifest.json exists at the root.
    zip.by_name("manifest.json")
        .expect("manifest.json must exist at archive root");
}

/// T02: export_project_backup manifest.json has schema_version=1 and
/// included_items.
#[test]
fn t02_export_backup_manifest_has_schema_version_and_included_items() {
    let temp = TempDir::new("t02");
    let project = create_project(temp.path(), "novel.optimizer", "Novel");
    let project_id = project.info().unwrap().project_id;
    let output_path = temp.join("backup.optimizer-backup");

    let response = project
        .export_project_backup(&ExportProjectBackupRequest {
            project_id: project_id.clone(),
            include_endpoints: false,
            include_recent: false,
            output_path: output_path.to_string_lossy().into_owned(),
            endpoints_json: None,
            recent_json: None,
        })
        .expect("export_project_backup failed");

    let manifest = &response.manifest;
    assert_eq!(manifest.schema_version, 1, "schema_version must be 1");
    assert_eq!(manifest.source_project_id, project_id);
    assert!(!manifest.source_path.is_empty());
    assert!(!manifest.generated_at.is_empty());
    assert_eq!(manifest.optimizer_version, "0.8.0");
    assert!(
        manifest.included_items.contains(&"project.sqlite3".to_string()),
        "included_items must contain project.sqlite3"
    );
    assert!(
        manifest.included_items.contains(&"manifest.json".to_string()),
        "included_items must contain manifest.json"
    );

    // Read manifest.json from the archive and verify it matches.
    let file = fs::File::open(&response.archive_path).unwrap();
    let mut zip = zip::ZipArchive::new(file).unwrap();
    let mut manifest_entry = zip.by_name("manifest.json").unwrap();
    let mut manifest_bytes = Vec::new();
    use std::io::Read as _;
    manifest_entry.read_to_end(&mut manifest_bytes).unwrap();
    let archived_manifest: optimizer_store::BackupManifest =
        serde_json::from_slice(&manifest_bytes).unwrap();
    assert_eq!(archived_manifest.schema_version, 1);
    assert_eq!(archived_manifest.source_project_id, project_id);
}

/// T03: include_endpoints=true saves endpoint metadata but no secrets.
#[test]
fn t03_export_backup_with_endpoints_saves_metadata_no_secret() {
    let temp = TempDir::new("t03");
    let project = create_project(temp.path(), "novel.optimizer", "Novel");
    let project_id = project.info().unwrap().project_id;
    let output_path = temp.join("backup.optimizer-backup");

    // Provide endpoint metadata (no secrets in the JSON).
    let endpoints_json = r#"[{"id":"ep-1","label":"DeepSeek","baseUrl":"https://api.deepseek.com","credentialRef":"ref-1"}]"#;

    let response = project
        .export_project_backup(&ExportProjectBackupRequest {
            project_id,
            include_endpoints: true,
            include_recent: false,
            output_path: output_path.to_string_lossy().into_owned(),
            endpoints_json: Some(endpoints_json.to_string()),
            recent_json: None,
        })
        .expect("export_project_backup failed");

    assert!(
        response.manifest.included_items.contains(&"endpoints.json".to_string()),
        "included_items must contain endpoints.json when include_endpoints=true"
    );

    // Verify endpoints.json exists in the archive and contains no API Key.
    let file = fs::File::open(&response.archive_path).unwrap();
    let mut zip = zip::ZipArchive::new(file).unwrap();
    let mut ep_entry = zip
        .by_name("endpoints.json")
        .expect("endpoints.json must exist in archive");
    let mut ep_bytes = Vec::new();
    use std::io::Read as _;
    ep_entry.read_to_end(&mut ep_bytes).unwrap();
    let ep_content = String::from_utf8(ep_bytes).unwrap();
    assert!(
        !ep_content.to_lowercase().contains("api_key"),
        "endpoints.json must not contain api_key"
    );
    assert!(
        ep_content.contains("credentialRef"),
        "endpoints.json must contain credentialRef (metadata, not secret)"
    );
}

/// T04: include_endpoints=false does not include endpoints.json.
#[test]
fn t04_export_backup_without_endpoints_omits_endpoints_json() {
    let temp = TempDir::new("t04");
    let project = create_project(temp.path(), "novel.optimizer", "Novel");
    let project_id = project.info().unwrap().project_id;
    let output_path = temp.join("backup.optimizer-backup");

    let response = project
        .export_project_backup(&ExportProjectBackupRequest {
            project_id,
            include_endpoints: false,
            include_recent: false,
            output_path: output_path.to_string_lossy().into_owned(),
            endpoints_json: None,
            recent_json: None,
        })
        .expect("export_project_backup failed");

    assert!(
        !response
            .manifest
            .included_items
            .contains(&"endpoints.json".to_string()),
        "included_items must NOT contain endpoints.json when include_endpoints=false"
    );

    let file = fs::File::open(&response.archive_path).unwrap();
    let mut zip = zip::ZipArchive::new(file).unwrap();
    let result = zip.by_name("endpoints.json");
    assert!(
        result.is_err(),
        "endpoints.json must NOT exist in archive when include_endpoints=false"
    );
}

/// T05: import_project_backup extracts and validates manifest + SQLite.
#[test]
fn t05_import_project_backup_validates_manifest_and_sqlite() {
    let temp = TempDir::new("t05");

    // Create and export a backup.
    let project = create_project(temp.path(), "source.optimizer", "Source");
    let project_id = project.info().unwrap().project_id;
    let backup_path = temp.join("backup.optimizer-backup");
    let export_response = project
        .export_project_backup(&ExportProjectBackupRequest {
            project_id: project_id.clone(),
            include_endpoints: false,
            include_recent: false,
            output_path: backup_path.to_string_lossy().into_owned(),
            endpoints_json: None,
            recent_json: None,
        })
        .expect("export failed");

    // Restore into a new directory.
    let restore_dir = temp.join("restore");
    fs::create_dir_all(&restore_dir).unwrap();
    let response = OpenedProject::import_project_backup(&ImportProjectBackupRequest {
        archive_path: export_response.archive_path.clone(),
        target_directory: restore_dir.to_string_lossy().into_owned(),
        new_project_id: None,
        overwrite: false,
    })
    .expect("import_project_backup failed");

    assert_eq!(response.schema_version, 1);
    assert_eq!(response.manifest.schema_version, 1);
    assert!(!response.project_id.is_empty());
    assert_ne!(
        response.project_id, project_id,
        "restored project_id must differ from original"
    );
    assert!(
        response.project_path.ends_with(".optimizer"),
        "project_path must end with .optimizer, got: {}",
        response.project_path
    );
    assert!(
        response.restored_items.contains(&"project.sqlite3".to_string()),
        "restored_items must contain project.sqlite3"
    );

    // Verify the restored project can be opened.
    let opened = OpenedProject::open(&response.project_path)
        .expect("restored project must open successfully");
    assert_eq!(opened.info().unwrap().project_id, response.project_id);
}

/// T06: import_project_backup generates a new project_id by default.
#[test]
fn t06_import_project_backup_generates_new_project_id_by_default() {
    let temp = TempDir::new("t06");
    let project = create_project(temp.path(), "source.optimizer", "Source");
    let project_id = project.info().unwrap().project_id;
    let backup_path = temp.join("backup.optimizer-backup");
    let export_response = project
        .export_project_backup(&ExportProjectBackupRequest {
            project_id: project_id.clone(),
            include_endpoints: false,
            include_recent: false,
            output_path: backup_path.to_string_lossy().into_owned(),
            endpoints_json: None,
            recent_json: None,
        })
        .expect("export failed");

    let restore_dir = temp.join("restore");
    fs::create_dir_all(&restore_dir).unwrap();
    let response = OpenedProject::import_project_backup(&ImportProjectBackupRequest {
        archive_path: export_response.archive_path,
        target_directory: restore_dir.to_string_lossy().into_owned(),
        new_project_id: None,
        overwrite: false,
    })
    .expect("import failed");

    assert_ne!(
        response.project_id, project_id,
        "default restore must generate a new project_id"
    );
    assert!(
        response.project_id.starts_with("project-"),
        "new project_id should follow the project-{{uuid}} format, got: {}",
        response.project_id
    );
}

/// T07: import_project_backup rejects mismatched schema_version.
#[test]
fn t07_import_project_backup_rejects_mismatched_schema_version() {
    let temp = TempDir::new("t07");
    let project = create_project(temp.path(), "source.optimizer", "Source");
    let project_id = project.info().unwrap().project_id;
    let backup_path = temp.join("backup.optimizer-backup");
    let export_response = project
        .export_project_backup(&ExportProjectBackupRequest {
            project_id,
            include_endpoints: false,
            include_recent: false,
            output_path: backup_path.to_string_lossy().into_owned(),
            endpoints_json: None,
            recent_json: None,
        })
        .expect("export failed");

    // Tamper with the manifest inside the archive to change schema_version.
    let file = fs::File::open(&export_response.archive_path).unwrap();
    let mut zip = zip::ZipArchive::new(file).unwrap();
    let manifest_bytes: Vec<u8> = {
        let mut manifest_entry = zip.by_name("manifest.json").unwrap();
        let mut buf = Vec::new();
        manifest_entry.read_to_end(&mut buf).unwrap();
        buf
    };
    let mut manifest_json: serde_json::Value = serde_json::from_slice(&manifest_bytes).unwrap();
    manifest_json["schemaVersion"] = serde_json::json!(999);
    let tampered_bytes = serde_json::to_vec_pretty(&manifest_json).unwrap();

    // Write a new archive with the tampered manifest.
    let tampered_path = temp.join("tampered.optimizer-backup");
    {
        let out_file = fs::File::create(&tampered_path).unwrap();
        let mut out_zip = zip::ZipWriter::new(out_file);
        let options =
            zip::write::SimpleFileOptions::default().compression_method(zip::CompressionMethod::Deflated);
        out_zip.start_file("manifest.json", options).unwrap();
        out_zip.write_all(&tampered_bytes).unwrap();

        // Copy project/ entries from the original archive.
        let original_file = fs::File::open(&export_response.archive_path).unwrap();
        let mut original_zip = zip::ZipArchive::new(original_file).unwrap();
        for i in 0..original_zip.len() {
            let mut entry = original_zip.by_index(i).unwrap();
            let name = entry.name().to_string();
            if !name.starts_with("project/") {
                continue;
            }
            if entry.is_dir() {
                out_zip.add_directory(&name, options).unwrap();
            } else {
                out_zip.start_file(&name, options).unwrap();
                let mut buf = Vec::new();
                std::io::Read::read_to_end(&mut entry, &mut buf).unwrap();
                out_zip.write_all(&buf).unwrap();
            }
        }
        out_zip.finish().unwrap();
    }

    let restore_dir = temp.join("restore");
    fs::create_dir_all(&restore_dir).unwrap();
    let result = OpenedProject::import_project_backup(&ImportProjectBackupRequest {
        archive_path: tampered_path.to_string_lossy().into_owned(),
        target_directory: restore_dir.to_string_lossy().into_owned(),
        new_project_id: None,
        overwrite: false,
    });

    assert!(
        result.is_err(),
        "import must reject mismatched schema_version"
    );
    let err_msg = format!("{:?}", result.unwrap_err());
    assert!(
        err_msg.contains("schema_version") || err_msg.contains("not supported"),
        "error must mention schema_version mismatch, got: {err_msg}"
    );
}

/// T08: import_project_backup with overwrite=false rejects existing target.
#[test]
fn t08_import_project_backup_rejects_existing_target_without_overwrite() {
    let temp = TempDir::new("t08");
    let project = create_project(temp.path(), "source.optimizer", "Source");
    let project_id = project.info().unwrap().project_id;
    let backup_path = temp.join("backup.optimizer-backup");
    let export_response = project
        .export_project_backup(&ExportProjectBackupRequest {
            project_id,
            include_endpoints: false,
            include_recent: false,
            output_path: backup_path.to_string_lossy().into_owned(),
            endpoints_json: None,
            recent_json: None,
        })
        .expect("export failed");

    // Create the target directory with an existing .optimizer package.
    let restore_dir = temp.join("restore");
    fs::create_dir_all(&restore_dir).unwrap();
    // Pre-create the target package to trigger the overwrite check.
    let target_package = restore_dir.join("source.optimizer");
    fs::create_dir_all(&target_package).unwrap();

    let result = OpenedProject::import_project_backup(&ImportProjectBackupRequest {
        archive_path: export_response.archive_path,
        target_directory: restore_dir.to_string_lossy().into_owned(),
        new_project_id: None,
        overwrite: false,
    });

    assert!(
        result.is_err(),
        "import must reject when target exists and overwrite=false"
    );
    let err_msg = format!("{:?}", result.unwrap_err());
    assert!(
        err_msg.contains("already exists"),
        "error must mention target already exists, got: {err_msg}"
    );
}

/// T09: import_project_backup response includes source="backup".
/// (The recent_projects registry marking is verified at the Tauri layer;
/// this integration test confirms the response contract carries the
/// `source: "backup"` field for the frontend to badge the project.)
#[test]
fn t09_import_project_backup_response_has_source_backup() {
    let temp = TempDir::new("t09");
    let project = create_project(temp.path(), "source.optimizer", "Source");
    let project_id = project.info().unwrap().project_id;
    let backup_path = temp.join("backup.optimizer-backup");
    let export_response = project
        .export_project_backup(&ExportProjectBackupRequest {
            project_id,
            include_endpoints: false,
            include_recent: false,
            output_path: backup_path.to_string_lossy().into_owned(),
            endpoints_json: None,
            recent_json: None,
        })
        .expect("export failed");

    let restore_dir = temp.join("restore");
    fs::create_dir_all(&restore_dir).unwrap();
    let response = OpenedProject::import_project_backup(&ImportProjectBackupRequest {
        archive_path: export_response.archive_path,
        target_directory: restore_dir.to_string_lossy().into_owned(),
        new_project_id: None,
        overwrite: false,
    })
    .expect("import failed");

    assert_eq!(
        response.source, "backup",
        "response.source must be 'backup' for restored projects"
    );
    assert!(
        response
            .warnings
            .iter()
            .any(|w| w.contains("Secret") || w.contains("API Key")),
        "warnings must mention that Secret/API Key is not included"
    );
}
