use std::io::{Read, Seek, Write};
use std::path::{Path, PathBuf};

use optimizer_store::BackupManifest;
use serde::Serialize;
use uuid::Uuid;

use super::*;

// ---------------------------------------------------------------------------
// v0.8.0 Stage 1 (FR-11) — project-level backup & restore
// ---------------------------------------------------------------------------

const BACKUP_MANIFEST_SCHEMA_VERSION: i64 = 1;
const BACKUP_OPTIMIZER_VERSION: &str = "0.8.0";
const BACKUP_ARCHIVE_EXTENSION: &str = ".optimizer-backup";
const BACKUP_PROJECT_PREFIX: &str = "project/";
const BACKUP_MANIFEST_FILE: &str = "manifest.json";
const BACKUP_ENDPOINTS_FILE: &str = "endpoints.json";
const BACKUP_RECENT_FILE: &str = "recent.json";
const BACKUP_DATABASE_FILE: &str = "project.sqlite3";
const BACKUP_MAX_ARCHIVE_BYTES: u64 = 2 * 1024 * 1024 * 1024; // 2 GiB safety cap
const BACKUP_MAX_EXTRACTED_BYTES: u64 = 4 * 1024 * 1024 * 1024; // 4 GiB zip-bomb guard

/// Request to export an entire `.optimizer` project package as a single
/// `.optimizer-backup` zip archive. The caller (Tauri command layer) is
/// responsible for providing endpoint metadata and recent-project metadata
/// as JSON strings — the host core never touches the Secret Store.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct ExportProjectBackupRequest {
    pub project_id: String,
    pub include_endpoints: bool,
    pub include_recent: bool,
    pub output_path: String,
    /// Serialized endpoint metadata (no secrets). When `include_endpoints`
    /// is `true` but this is `None`, an empty array `"[]"` is written.
    pub endpoints_json: Option<String>,
    /// Serialized recent-project metadata for this project. When
    /// `include_recent` is `true` but this is `None`, the field is omitted
    /// from the archive.
    pub recent_json: Option<String>,
}

#[derive(Debug, Clone, PartialEq, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct ExportProjectBackupResponse {
    pub schema_version: u32,
    pub archive_path: String,
    pub manifest: BackupManifest,
    pub bytes_written: u64,
    pub item_count: usize,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct ImportProjectBackupRequest {
    pub archive_path: String,
    pub target_directory: String,
    pub new_project_id: Option<String>,
    pub overwrite: bool,
}

#[derive(Debug, Clone, PartialEq, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct ImportProjectBackupResponse {
    pub schema_version: u32,
    pub project_id: String,
    pub project_path: String,
    pub manifest: BackupManifest,
    pub restored_items: Vec<String>,
    pub warnings: Vec<String>,
    /// Always `"backup"` for restores from a `.optimizer-backup` archive.
    /// The frontend uses this to badge the project in the recent-projects
    /// list (contract T09).
    pub source: String,
}

/// Walk the project package root and add every file to the zip writer
/// under the `project/` prefix. Returns the number of files added.
fn add_directory_to_zip<W: Write + Seek>(
    zip: &mut zip::ZipWriter<W>,
    options: zip::write::SimpleFileOptions,
    root: &Path,
    prefix: &str,
    excluded_root_dir_names: &[&str],
) -> Result<usize, WorkspaceCommandError> {
    let mut count = 0usize;
    let mut stack = vec![std::path::PathBuf::from("")];
    while let Some(relative) = stack.pop() {
        let absolute = root.join(&relative);
        let entries = std::fs::read_dir(&absolute).map_err(|source| {
            WorkspaceCommandError::Validation(format!(
                "failed to read directory {}: {source}",
                absolute.display()
            ))
        })?;
        for entry in entries.flatten() {
            let entry_path = entry.path();
            let entry_relative = relative.join(entry.file_name());
            let zip_name = format!("{prefix}/{}", entry_relative.to_string_lossy());
            let metadata = entry.metadata().map_err(|source| {
                WorkspaceCommandError::Validation(format!(
                    "failed to inspect {}: {source}",
                    entry_path.display()
                ))
            })?;
            if metadata.is_dir() {
                // Skip top-level directories that must never be bundled
                // (e.g. the migration `backups/` folder, which can be very
                // large and is fully rebuildable).
                if relative.as_os_str().is_empty()
                    && entry
                        .file_name()
                        .to_str()
                        .is_some_and(|name| excluded_root_dir_names.contains(&name))
                {
                    continue;
                }
                zip.add_directory(&zip_name, options)
                    .map_err(|source| WorkspaceCommandError::Validation(
                        format!("failed to add directory to archive: {source}"),
                    ))?;
                stack.push(entry_relative);
                count += 1;
            } else if metadata.is_file() {
                let file_bytes = std::fs::read(&entry_path).map_err(|source| {
                    WorkspaceCommandError::Validation(format!(
                        "failed to read file {}: {source}",
                        entry_path.display()
                    ))
                })?;
                zip.start_file(&zip_name, options)
                    .map_err(|source| WorkspaceCommandError::Validation(
                        format!("failed to start file in archive: {source}"),
                    ))?;
                zip.write_all(&file_bytes).map_err(|source| {
                    WorkspaceCommandError::Validation(format!(
                        "failed to write file to archive: {source}"
                    ))
                })?;
                count += 1;
            }
        }
    }
    Ok(count)
}

pub(crate) fn export_project_backup(
    store: &OptimizerStore,
    project_root: &Path,
    request: &ExportProjectBackupRequest,
) -> Result<ExportProjectBackupResponse, WorkspaceCommandError> {
    if request.project_id.trim().is_empty() {
        return Err(WorkspaceCommandError::Validation(
            "project_id must be a non-empty string".into(),
        ));
    }
    if request.output_path.trim().is_empty() {
        return Err(WorkspaceCommandError::Validation(
            "output_path must be a non-empty string".into(),
        ));
    }

    let output_path = Path::new(&request.output_path);
    if !output_path.is_absolute() {
        return Err(WorkspaceCommandError::Validation(
            "output_path must be absolute".into(),
        ));
    }

    // Ensure the archive ends with `.optimizer-backup`.
    let final_output = if output_path
        .extension()
        .and_then(|ext| ext.to_str())
        .map(|ext| ext == "optimizer-backup")
        .unwrap_or(false)
    {
        output_path.to_path_buf()
    } else {
        let mut path = output_path.as_os_str().to_os_string();
        path.push(BACKUP_ARCHIVE_EXTENSION);
        PathBuf::from(path)
    };

    // Create parent directory if needed.
    if let Some(parent) = final_output.parent()
        && !parent.as_os_str().is_empty()
        && !parent.exists()
    {
        std::fs::create_dir_all(parent).map_err(|source| {
            WorkspaceCommandError::Validation(format!(
                "failed to create output parent directory: {source}"
            ))
        })?;
    }

    let generated_at = now()?;
    let db_version = store
        .diagnostics()
        .map_err(WorkspaceCommandError::from)?
        .schema_version;

    let mut included_items = vec![
        "project.sqlite3".to_string(),
        "manifest.json".to_string(),
        "assets".to_string(),
        // `backups/` (pre-migration snapshots) is deliberately excluded: it
        // can be very large and is rebuildable. `exports/` remains included.
        "exports".to_string(),
    ];
    if request.include_endpoints {
        included_items.push("endpoints.json".to_string());
    }
    if request.include_recent {
        included_items.push("recent.json".to_string());
    }

    let manifest = BackupManifest {
        schema_version: BACKUP_MANIFEST_SCHEMA_VERSION,
        source_project_id: request.project_id.clone(),
        source_path: project_root.to_string_lossy().into_owned(),
        generated_at: generated_at.clone(),
        included_items: included_items.clone(),
        optimizer_version: BACKUP_OPTIMIZER_VERSION.into(),
        schema_db_version: db_version,
    };

    let manifest_json = serde_json::to_vec_pretty(&manifest)
        .map_err(|source| WorkspaceCommandError::Validation(
            format!("failed to serialize backup manifest: {source}"),
        ))?;

    let endpoints_bytes = if request.include_endpoints {
        Some(
            request
                .endpoints_json
                .clone()
                .unwrap_or_else(|| "[]".to_string())
                .into_bytes(),
        )
    } else {
        None
    };

    let recent_bytes = if request.include_recent {
        request.recent_json.clone().map(|s| s.into_bytes())
    } else {
        None
    };

    // Write to a temporary file first, then rename for atomicity.
    let temp_path = final_output.with_extension("optimizer-backup.tmp");
    if temp_path.exists() {
        let _ = std::fs::remove_file(&temp_path);
    }

    let file = std::fs::File::create(&temp_path).map_err(|source| {
        WorkspaceCommandError::Validation(format!(
            "failed to create temporary archive file: {source}"
        ))
    })?;
    let mut zip = zip::ZipWriter::new(file);
    let options = zip::write::SimpleFileOptions::default()
        .compression_method(zip::CompressionMethod::Deflated);

    // Write manifest.json at the archive root.
    zip.start_file(BACKUP_MANIFEST_FILE, options)
        .map_err(|source| WorkspaceCommandError::Validation(
            format!("failed to start manifest in archive: {source}"),
        ))?;
    zip.write_all(&manifest_json)
        .map_err(|source| WorkspaceCommandError::Validation(
            format!("failed to write manifest to archive: {source}"),
        ))?;

    // Optional endpoints.json at the archive root.
    if let Some(ep_bytes) = &endpoints_bytes {
        zip.start_file(BACKUP_ENDPOINTS_FILE, options)
            .map_err(|source| WorkspaceCommandError::Validation(
                format!("failed to start endpoints in archive: {source}"),
            ))?;
        zip.write_all(ep_bytes)
            .map_err(|source| WorkspaceCommandError::Validation(
                format!("failed to write endpoints to archive: {source}"),
            ))?;
    }

    // Optional recent.json at the archive root.
    if let Some(recent) = &recent_bytes {
        zip.start_file(BACKUP_RECENT_FILE, options)
            .map_err(|source| WorkspaceCommandError::Validation(
                format!("failed to start recent metadata in archive: {source}"),
            ))?;
        zip.write_all(recent)
            .map_err(|source| WorkspaceCommandError::Validation(
                format!("failed to write recent metadata to archive: {source}"),
            ))?;
    }

    // Add the entire project directory under `project/`, excluding the
    // rebuildable migration `backups/` folder.
    let file_count =
        add_directory_to_zip(&mut zip, options, project_root, "project", &["backups"])?;

    zip.finish()
        .map_err(|source| WorkspaceCommandError::Validation(
            format!("failed to finalize backup archive: {source}"),
        ))?;

    let bytes_written = std::fs::metadata(&temp_path)
        .map(|m| m.len())
        .unwrap_or(0);
    if bytes_written > BACKUP_MAX_ARCHIVE_BYTES {
        let _ = std::fs::remove_file(&temp_path);
        return Err(WorkspaceCommandError::Validation(
            "backup archive exceeds maximum size".into(),
        ));
    }

    // Atomic rename.
    if final_output.exists() {
        let _ = std::fs::remove_file(&final_output);
    }
    std::fs::rename(&temp_path, &final_output).map_err(|source| {
        let _ = std::fs::remove_file(&temp_path);
        WorkspaceCommandError::Validation(format!(
            "failed to publish backup archive: {source}"
        ))
    })?;

    Ok(ExportProjectBackupResponse {
        schema_version: 1,
        archive_path: final_output.to_string_lossy().into_owned(),
        manifest,
        bytes_written,
        item_count: file_count,
    })
}

/// Guard against zip-slip: ensure `entry_name` normalizes to a path inside
/// `target` with no `..` traversal or absolute escape.
fn safe_extract_path(target: &Path, entry_name: &str) -> Option<PathBuf> {
    // Normalize backslashes (Windows archives may use them).
    let normalized = entry_name.replace('\\', "/");
    let path = Path::new(&normalized);
    if path.is_absolute() {
        return None;
    }
    let mut result = target.to_path_buf();
    for component in path.components() {
        match component {
            std::path::Component::Normal(part) => result.push(part),
            std::path::Component::CurDir => {}
            _ => return None, // reject ParentDir, RootDir, Prefix
        }
    }
    // Final check: the resolved path must be inside target.
    if result.starts_with(target) {
        Some(result)
    } else {
        None
    }
}

/// Determine the package folder name from the backup manifest's `source_path`.
/// Falls back to `Restored.optimizer` if the source path has no usable name.
fn package_folder_name_from_manifest(manifest: &BackupManifest) -> String {
    let path = Path::new(&manifest.source_path);
    if let Some(name) = path.file_name().and_then(|n| n.to_str())
        && !name.is_empty()
        && name.ends_with(".optimizer")
    {
        return name.to_string();
    }
    // Fall back to the directory name + .optimizer suffix.
    if let Some(parent) = path.file_name().and_then(|n| n.to_str())
        && !parent.is_empty()
    {
        return format!("{parent}.optimizer");
    }
    "Restored.optimizer".to_string()
}

/// Quote a SQL identifier for use inside double quotes, escaping any
/// embedded double quotes per the SQLite identifier rule (`"` -> `""`).
///
/// The names fed to this helper come from `sqlite_master` in the *archived*
/// database, which is attacker-influenced, so they must never be spliced
/// into SQL verbatim.
pub(crate) fn quote_sql_identifier(identifier: &str) -> String {
    format!("\"{}\"", identifier.replace('"', "\"\""))
}

/// Quote a SQL string literal inside single quotes, escaping any embedded
/// single quotes per the SQLite literal rule (`'` -> `''`).
pub(crate) fn quote_sql_string_literal(value: &str) -> String {
    format!("'{}'", value.replace('\'', "''"))
}

/// Mirrors the IPC-side `is_safe_binding_id` guard: an id must be 1..=128
/// ASCII alphanumeric characters with only `_` / `-` separators, so it can be
/// safely embedded in SQL identifiers and filesystem paths.
pub(crate) fn is_safe_binding_id(value: &str) -> bool {
    !value.is_empty()
        && value.len() <= 128
        && value
            .bytes()
            .all(|byte| byte.is_ascii_alphanumeric() || matches!(byte, b'_' | b'-'))
}

/// Remap `project_id` across all SQLite tables that carry the column.
/// Uses a dynamic schema query to stay resilient to future migrations.
///
/// Two obstacles must be overcome:
///
/// 1. **Foreign-key constraints** — `project.id` is the PK parent for 13+
///    child tables. Updating the parent row first would trip
///    `FOREIGN KEY constraint failed` while children still reference the
///    old id. We use `PRAGMA defer_foreign_keys = ON` inside a transaction
///    so FK checks are deferred until `COMMIT`, at which point every table
///    has been updated.
///
/// 2. **Immutable triggers** — Tables like `commit_node`, `edit_journal`,
///    `change_set`, etc. carry `BEFORE UPDATE` triggers that
///    `RAISE(ABORT, 'immutable:<table>')` on any UPDATE. We temporarily
///    drop these triggers (saving their SQL), perform the remap, and
///    recreate them before `COMMIT` so the schema is fully restored.
///
/// The whole remap runs inside a single rusqlite `Transaction` opened with
/// `TransactionBehavior::Immediate`; on any error the transaction is
/// rolled back (including on early `?` returns via `Drop`).
pub(crate) fn remap_project_id(
    connection: &mut rusqlite::Connection,
    old_id: &str,
    new_id: &str,
) -> Result<usize, WorkspaceCommandError> {
    // Collect all immutable trigger definitions so we can restore them.
    let immutable_triggers: Vec<(String, String)> = {
        let mut stmt = connection
            .prepare(
                "SELECT name, sql FROM sqlite_master \
                 WHERE type = 'trigger' AND name LIKE '%_immutable_%'",
            )
            .map_err(|source| WorkspaceCommandError::Store(StoreError::Sqlite(source)))?;
        let rows = stmt
            .query_map([], |row| {
                Ok((row.get::<_, String>(0)?, row.get::<_, String>(1)?))
            })
            .map_err(|source| WorkspaceCommandError::Store(StoreError::Sqlite(source)))?;
        rows.filter_map(|r| r.ok()).collect()
    };

    let transaction = connection
        .transaction_with_behavior(rusqlite::TransactionBehavior::Immediate)
        .map_err(|source| WorkspaceCommandError::Store(StoreError::Sqlite(source)))?;

    // Explicitly enable deferred foreign key checks inside this transaction,
    // then verify the pragma actually took effect before mutating data.
    transaction
        .execute_batch("PRAGMA defer_foreign_keys = ON")
        .map_err(|source| WorkspaceCommandError::Store(StoreError::Sqlite(source)))?;
    let defer_enabled: i64 = transaction
        .query_row("PRAGMA defer_foreign_keys", [], |row| row.get(0))
        .map_err(|source| WorkspaceCommandError::Store(StoreError::Sqlite(source)))?;
    if defer_enabled != 1 {
        return Err(WorkspaceCommandError::Validation(
            "deferred foreign key checks could not be enabled for project remap".into(),
        ));
    }

    // Temporarily drop all immutable triggers so we can UPDATE the
    // immutable tables without tripping RAISE(ABORT). Identifiers are
    // double-quoted and escaped (they originate from the archived DB).
    for (name, _) in &immutable_triggers {
        let sql = format!(
            "DROP TRIGGER IF EXISTS {}",
            quote_sql_identifier(name)
        );
        transaction
            .execute_batch(&sql)
            .map_err(|source| WorkspaceCommandError::Store(StoreError::Sqlite(source)))?;
    }

    // Update the canonical `project` row.
    let updated = transaction
        .execute(
            "UPDATE project SET id = ?1 WHERE id = ?2",
            rusqlite::params![new_id, old_id],
        )
        .map_err(|source| WorkspaceCommandError::Store(StoreError::Sqlite(source)))?;
    if updated == 0 {
        return Err(WorkspaceCommandError::Store(StoreError::NotFound {
            entity: "project",
            id: old_id.to_string(),
        }));
    }

    // Find all tables (except `project`) that have a `project_id` column.
    // Scoped in a block so the borrowed `Statement`/rows are dropped before
    // `transaction.commit()` consumes the transaction below.
    let tables: Vec<String> = {
        let mut tables: Vec<String> = Vec::new();
        let mut stmt = transaction
            .prepare(
                "SELECT name FROM sqlite_master WHERE type = 'table' AND name NOT IN ('project','schema_migration')",
            )
            .map_err(|source| {
                WorkspaceCommandError::Validation(format!(
                    "failed to query table list: {source}"
                ))
            })?;
        let table_rows = stmt
            .query_map([], |row| row.get::<_, String>(0))
            .map_err(|source| {
                WorkspaceCommandError::Validation(format!(
                    "failed to read table list: {source}"
                ))
            })?;
        for table_name in table_rows.flatten() {
            // Check if this table has a `project_id` column. `pragma_table_info`
            // takes a *string literal* argument (not an identifier), so it is
            // single-quoted and escaped.
            let sql = format!(
                "SELECT COUNT(*) > 0 FROM pragma_table_info({}) WHERE name = 'project_id'",
                quote_sql_string_literal(&table_name)
            );
            let has_project_id: bool = transaction
                .query_row(&sql, [], |row| row.get(0))
                .map_err(|source| WorkspaceCommandError::Store(StoreError::Sqlite(source)))?;
            if has_project_id {
                tables.push(table_name);
            }
        }
        tables
    };

    let mut total = updated;
    for table in tables {
        let sql = format!(
            "UPDATE {} SET project_id = ?1 WHERE project_id = ?2",
            quote_sql_identifier(&table)
        );
        let changed = transaction
            .execute(&sql, rusqlite::params![new_id, old_id])
            .map_err(|source| WorkspaceCommandError::Store(StoreError::Sqlite(source)))?;
        total += changed;
    }

    // Special case: summary_invalidation.scope_id stores the project_id
    // verbatim when scope_type = 'project'. The generic loop above only
    // updates columns named 'project_id', so scope_id is left stale and
    // the verify_invariants() check
    //   `si.scope_type = 'project' AND si.scope_id <> si.project_id`
    // would fail.
    let scope_changed = transaction
        .execute(
            "UPDATE summary_invalidation SET scope_id = ?1 \
             WHERE scope_type = 'project' AND scope_id = ?2",
            rusqlite::params![new_id, old_id],
        )
        .map_err(|source| WorkspaceCommandError::Store(StoreError::Sqlite(source)))?;
    total += scope_changed;

    // Recreate all immutable triggers before COMMIT so the schema is
    // fully restored.
    for (_, sql) in &immutable_triggers {
        transaction
            .execute_batch(sql)
            .map_err(|source| WorkspaceCommandError::Store(StoreError::Sqlite(source)))?;
    }

    // Commit — deferred FK constraints are checked here. If any child
    // table still references a non-existent project, this will fail.
    transaction
        .commit()
        .map_err(|source| WorkspaceCommandError::Store(StoreError::Sqlite(source)))?;

    Ok(total)
}

pub(crate) fn import_project_backup(
    request: &ImportProjectBackupRequest,
) -> Result<ImportProjectBackupResponse, WorkspaceCommandError> {
    if request.archive_path.trim().is_empty() {
        return Err(WorkspaceCommandError::Validation(
            "archive_path must be a non-empty string".into(),
        ));
    }
    if request.target_directory.trim().is_empty() {
        return Err(WorkspaceCommandError::Validation(
            "target_directory must be a non-empty string".into(),
        ));
    }

    let archive_path = Path::new(&request.archive_path);
    if !archive_path.is_absolute() {
        return Err(WorkspaceCommandError::Validation(
            "archive_path must be absolute".into(),
        ));
    }
    if !archive_path.is_file() {
        return Err(WorkspaceCommandError::Validation(
            "archive_path does not point to an existing file".into(),
        ));
    }

    let target_parent = Path::new(&request.target_directory);
    if !target_parent.is_absolute() {
        return Err(WorkspaceCommandError::Validation(
            "target_directory must be absolute".into(),
        ));
    }
    if !target_parent.is_dir() {
        return Err(WorkspaceCommandError::Validation(
            "target_directory does not point to an existing directory".into(),
        ));
    }

    // Validate new_project_id if provided.
    if let Some(ref new_id) = request.new_project_id
        && !is_safe_binding_id(new_id)
    {
        return Err(WorkspaceCommandError::Validation(
            "new_project_id must be 1..=128 ASCII letters, digits, '_' or '-'".into(),
        ));
    }

    // Open the zip archive.
    let file = std::fs::File::open(archive_path).map_err(|source| {
        WorkspaceCommandError::Validation(format!("failed to open backup archive: {source}"))
    })?;
    let mut zip = zip::ZipArchive::new(file).map_err(|source| {
        WorkspaceCommandError::Validation(format!("failed to read backup archive: {source}"))
    })?;

    // Read manifest.json from the archive root. The `ZipFile` borrow of `zip`
    // is scoped so it is released before the entries are enumerated below.
    let manifest_bytes: Vec<u8> = {
        let mut entry = zip
            .by_name(BACKUP_MANIFEST_FILE)
            .map_err(|_| WorkspaceCommandError::Validation(
                "backup archive is missing manifest.json at root".into(),
            ))?;
        let mut buffer = Vec::new();
        entry.read_to_end(&mut buffer).map_err(|source| {
            WorkspaceCommandError::Validation(format!(
                "failed to read manifest.json from archive: {source}"
            ))
        })?;
        buffer
    };
    let manifest: BackupManifest = serde_json::from_slice(&manifest_bytes).map_err(|source| {
        WorkspaceCommandError::Validation(format!(
            "failed to parse backup manifest: {source}"
        ))
    })?;

    // Validate schema_version (T07).
    if manifest.schema_version != BACKUP_MANIFEST_SCHEMA_VERSION {
        return Err(WorkspaceCommandError::Validation(format!(
            "backup manifest schema_version {} is not supported (expected {})",
            manifest.schema_version, BACKUP_MANIFEST_SCHEMA_VERSION
        )));
    }

    // Determine the package folder name and final path.
    let package_name = package_folder_name_from_manifest(&manifest);
    let final_path = target_parent.join(&package_name);

    // Check overwrite (T08).
    if final_path.exists() && !request.overwrite {
        return Err(WorkspaceCommandError::Validation(format!(
            "target package already exists: {} (set overwrite=true to replace)",
            final_path.display()
        )));
    }

    // Create a staging directory in the target parent.
    let staging_name = format!(".{package_name}.restoring-{}", Uuid::new_v4().simple());
    let staging_path = target_parent.join(&staging_name);
    std::fs::create_dir_all(&staging_path).map_err(|source| {
        WorkspaceCommandError::Validation(format!(
            "failed to create staging directory: {source}"
        ))
    })?;

    // Extract all entries from the archive.
    let mut extracted_bytes: u64 = 0;
    let mut restored_items: Vec<String> = Vec::new();
    let mut has_database = false;

    for index in 0..zip.len() {
        let mut entry = zip
            .by_index(index)
            .map_err(|source| WorkspaceCommandError::Validation(
                format!("failed to read archive entry: {source}"),
            ))?;
        let entry_name = entry.name().to_string();

        // Only extract entries under the `project/` prefix (or the manifest).
        if !entry_name.starts_with(BACKUP_PROJECT_PREFIX) {
            continue;
        }

        let relative = &entry_name[BACKUP_PROJECT_PREFIX.len()..];
        let target_path = match safe_extract_path(&staging_path, relative) {
            Some(p) => p,
            None => {
                let _ = std::fs::remove_dir_all(&staging_path);
                return Err(WorkspaceCommandError::Validation(format!(
                    "archive contains unsafe path: {entry_name}"
                )));
            }
        };

        if entry.is_dir() {
            std::fs::create_dir_all(&target_path).map_err(|source| {
                let _ = std::fs::remove_dir_all(&staging_path);
                WorkspaceCommandError::Validation(format!(
                    "failed to create directory {}: {source}",
                    target_path.display()
                ))
            })?;
            continue;
        }

        // Ensure parent directory exists.
        if let Some(parent) = target_path.parent() {
            std::fs::create_dir_all(parent).map_err(|source| {
                let _ = std::fs::remove_dir_all(&staging_path);
                WorkspaceCommandError::Validation(format!(
                    "failed to create parent directory: {source}"
                ))
            })?;
        }

        // Zip-bomb guard: track total extracted bytes.
        let file_size = entry.size();
        extracted_bytes = extracted_bytes.saturating_add(file_size);
        if extracted_bytes > BACKUP_MAX_EXTRACTED_BYTES {
            let _ = std::fs::remove_dir_all(&staging_path);
            return Err(WorkspaceCommandError::Validation(
                "backup archive exceeds maximum extracted size".into(),
            ));
        }

        let mut buffer = Vec::with_capacity(file_size as usize);
        entry.read_to_end(&mut buffer).map_err(|source| {
            let _ = std::fs::remove_dir_all(&staging_path);
            WorkspaceCommandError::Validation(format!(
                "failed to read file from archive: {source}"
            ))
        })?;
        std::fs::write(&target_path, &buffer).map_err(|source| {
            let _ = std::fs::remove_dir_all(&staging_path);
            WorkspaceCommandError::Validation(format!(
                "failed to write extracted file: {source}"
            ))
        })?;

        let file_name = target_path
            .file_name()
            .and_then(|n| n.to_str())
            .unwrap_or("");
        if file_name == BACKUP_DATABASE_FILE {
            has_database = true;
            restored_items.push("project.sqlite3".to_string());
        } else if file_name == "manifest.json" {
            restored_items.push("manifest.json".to_string());
        }
    }

    if !has_database {
        let _ = std::fs::remove_dir_all(&staging_path);
        return Err(WorkspaceCommandError::Validation(
            "backup archive does not contain a project database".into(),
        ));
    }

    // Validate SQLite invariants (T05).
    let database_path = staging_path.join(BACKUP_DATABASE_FILE);
    let mut store = OptimizerStore::open(&database_path)
        .map_err(|source| {
            let _ = std::fs::remove_dir_all(&staging_path);
            WorkspaceCommandError::from(source)
        })?;
    store
        .verify_invariants()
        .map_err(|source| {
            let _ = std::fs::remove_dir_all(&staging_path);
            WorkspaceCommandError::from(source)
        })?;

    // Determine the final project_id (T06).
    let original_project_id = manifest.source_project_id.clone();
    let new_project_id = request
        .new_project_id
        .clone()
        .unwrap_or_else(|| format!("project-{}", Uuid::new_v4().simple()));

    let mut warnings = Vec::new();
    if new_project_id != original_project_id {
        // Remap project_id in all SQLite tables.
        remap_project_id(store.connection_mut(), &original_project_id, &new_project_id)
            .inspect_err(|_| {
                let _ = std::fs::remove_dir_all(&staging_path);
            })?;

        // Update the project package manifest.json with the new project_id.
        let manifest_path = staging_path.join("manifest.json");
        let manifest_bytes = std::fs::read(&manifest_path).map_err(|source| {
            let _ = std::fs::remove_dir_all(&staging_path);
            WorkspaceCommandError::Validation(format!(
                "failed to read project manifest: {source}"
            ))
        })?;
        let mut project_manifest: serde_json::Value =
            serde_json::from_slice(&manifest_bytes).map_err(|source| {
                let _ = std::fs::remove_dir_all(&staging_path);
                WorkspaceCommandError::Validation(format!(
                    "failed to parse project manifest: {source}"
                ))
            })?;
        project_manifest["projectId"] = serde_json::json!(new_project_id);
        let updated_bytes = serde_json::to_vec_pretty(&project_manifest).map_err(|source| {
            let _ = std::fs::remove_dir_all(&staging_path);
            WorkspaceCommandError::Validation(format!(
                "failed to serialize updated manifest: {source}"
            ))
        })?;
        std::fs::write(&manifest_path, updated_bytes).map_err(|source| {
            let _ = std::fs::remove_dir_all(&staging_path);
            WorkspaceCommandError::Validation(format!(
                "failed to write updated manifest: {source}"
            ))
        })?;

        warnings.push(format!(
            "project_id remapped from {} to {}",
            original_project_id, new_project_id
        ));
    }

    // Add standard warnings.
    warnings.push(
        "Secret (API Key) is not included in the backup — re-enter credentials after restore"
            .to_string(),
    );

    // Re-validate after remap.
    drop(store);
    let store = OptimizerStore::open(&database_path).map_err(|source| {
        let _ = std::fs::remove_dir_all(&staging_path);
        WorkspaceCommandError::from(source)
    })?;
    store
        .verify_invariants()
        .map_err(|source| {
            let _ = std::fs::remove_dir_all(&staging_path);
            WorkspaceCommandError::from(source)
        })?;

    // Release the SQLite connection before any filesystem move.
    // On Windows, an open file handle prevents `rename` with
    // ERROR_ACCESS_DENIED (os error 5).
    drop(store);

    // If target exists and overwrite=true, remove it.
    if final_path.exists() && request.overwrite {
        std::fs::remove_dir_all(&final_path).map_err(|source| {
            let _ = std::fs::remove_dir_all(&staging_path);
            WorkspaceCommandError::Validation(format!(
                "failed to remove existing target: {source}"
            ))
        })?;
    }

    // Move staging to final. On Windows this may fail if any file handle
    // is still being released; retry a few times with a short delay.
    let mut rename_err = None;
    for attempt in 0..5u32 {
        match std::fs::rename(&staging_path, &final_path) {
            Ok(()) => {
                rename_err = None;
                break;
            }
            Err(source) => {
                rename_err = Some(source);
                if attempt < 4 {
                    std::thread::sleep(std::time::Duration::from_millis(50));
                }
            }
        }
    }
    if let Some(source) = rename_err {
        let _ = std::fs::remove_dir_all(&staging_path);
        return Err(WorkspaceCommandError::Validation(format!(
            "failed to publish restored project: {source}"
        )));
    }

    let updated_manifest = BackupManifest {
        source_project_id: new_project_id.clone(),
        ..manifest
    };

    Ok(ImportProjectBackupResponse {
        schema_version: 1,
        project_id: new_project_id,
        project_path: final_path.to_string_lossy().into_owned(),
        manifest: updated_manifest,
        restored_items,
        warnings,
        source: "backup".to_string(),
    })
}
