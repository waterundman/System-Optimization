use super::*;

impl OpenedProject {
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

    pub fn export_json(&self) -> Result<ExportJsonResponse, ProjectPackageError> {
        let snapshot = self
            .operations
            .store()
            .head_snapshot(&self.project_id)
            .map_err(ProjectPackageError::Store)?;
        let export = snapshot.to_export();
        let json = serde_json::to_string_pretty(&export).map_err(|source| ProjectPackageError::Io {
            operation: "serialize JSON export",
            source: std::io::Error::other(source),
        })?;

        let exports = self
            .root
            .resolve_for_create("exports")
            .map_err(ProjectPackageError::Host)?;
        fs::create_dir_all(&exports).map_err(|source| ProjectPackageError::Io {
            operation: "create exports directory",
            source,
        })?;
        let file_name = format!("optimizer-export-{}.json", Uuid::new_v4().simple());
        let relative = Path::new("exports").join(&file_name);
        let final_path = self
            .root
            .resolve_for_create(&relative)
            .map_err(ProjectPackageError::Host)?;
        let temporary = self
            .root
            .resolve_for_create(Path::new("exports").join(format!(".{file_name}.tmp")))
            .map_err(ProjectPackageError::Host)?;
        fs::write(&temporary, json.as_bytes()).map_err(|source| ProjectPackageError::Io {
            operation: "write JSON export",
            source,
        })?;
        if let Err(source) = fs::rename(&temporary, &final_path) {
            let _ = fs::remove_file(&temporary);
            return Err(ProjectPackageError::Io {
                operation: "publish JSON export",
                source,
            });
        }
        Ok(ExportJsonResponse {
            schema_version: export.schema_version,
            path: final_path.to_string_lossy().into_owned(),
        })
    }

    pub fn export_diagnostics(&self) -> Result<ExportDiagnosticsResponse, ProjectPackageError> {
        let project = self
            .operations
            .store()
            .get_project(&self.project_id)
            .map_err(ProjectPackageError::Store)?;
        let insights = self
            .operations
            .store()
            .get_operation_insights(&self.project_id)
            .map_err(ProjectPackageError::Store)?;
        let revision_metrics = self
            .operations
            .store()
            .get_revision_metrics(&self.project_id)
            .map_err(ProjectPackageError::Store)?;
        let payload_hash_variations = self
            .operations
            .store()
            .get_payload_hash_variations(&self.project_id)
            .map_err(ProjectPackageError::Store)?;
        let recent_run_records = self
            .operations
            .store()
            .list_operation_runs(&self.project_id, 50, 0)
            .map_err(ProjectPackageError::Store)?;
        let recent_runs = recent_run_records
            .into_iter()
            .map(RecentRunSummary::from)
            .collect();
        let generated_at = OffsetDateTime::now_utc()
            .format(&Rfc3339)
            .map_err(|_| ProjectPackageError::Clock)?;
        let manifest = DiagnosticManifest {
            project_id: self.project_id.clone(),
            project_title: project.title,
            generated_at,
            store_schema_version: self.database_schema_version as u32,
        };
        let bundle = DiagnosticBundle {
            schema_version: 4,
            manifest,
            insights,
            recent_runs,
            revision_metrics,
            payload_hash_variations,
        };
        let json = serde_json::to_string_pretty(&bundle).map_err(|source| ProjectPackageError::Io {
            operation: "serialize diagnostics export",
            source: std::io::Error::other(source),
        })?;

        let exports = self
            .root
            .resolve_for_create("exports")
            .map_err(ProjectPackageError::Host)?;
        fs::create_dir_all(&exports).map_err(|source| ProjectPackageError::Io {
            operation: "create exports directory",
            source,
        })?;
        let file_name = format!("optimizer-diagnostics-{}.json", Uuid::new_v4().simple());
        let relative = Path::new("exports").join(&file_name);
        let final_path = self
            .root
            .resolve_for_create(&relative)
            .map_err(ProjectPackageError::Host)?;
        let temporary = self
            .root
            .resolve_for_create(Path::new("exports").join(format!(".{file_name}.tmp")))
            .map_err(ProjectPackageError::Host)?;
        fs::write(&temporary, json.as_bytes()).map_err(|source| ProjectPackageError::Io {
            operation: "write diagnostics export",
            source,
        })?;
        if let Err(source) = fs::rename(&temporary, &final_path) {
            let _ = fs::remove_file(&temporary);
            return Err(ProjectPackageError::Io {
                operation: "publish diagnostics export",
                source,
            });
        }
        Ok(ExportDiagnosticsResponse {
            schema_version: 1,
            path: final_path.to_string_lossy().into_owned(),
        })
    }
}
