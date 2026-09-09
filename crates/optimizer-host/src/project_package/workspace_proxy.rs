use super::*;

impl OpenedProject {
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

    pub fn archived_documents(&self) -> Result<Vec<ArchivedDocument>, WorkspaceCommandError> {
        list_archived_documents(self.operations.store(), &self.project_id)
    }

    pub fn summary_invalidations(&self) -> Result<Vec<SummaryInvalidation>, WorkspaceCommandError> {
        list_summary_invalidations(self.operations.store(), &self.project_id)
    }

    pub fn refresh_summaries(
        &mut self,
        spec: &RefreshSummariesSpec,
    ) -> Result<SummaryRefreshReport, WorkspaceCommandError> {
        refresh_summaries(self.operations.store_mut(), &self.project_id, spec)
    }

    pub fn summary_context(
        &self,
        spec: &SummaryContextSpec,
    ) -> Result<Vec<SummaryContextCandidate>, WorkspaceCommandError> {
        summary_context(self.operations.store(), &self.project_id, spec)
    }

    pub fn rename_document(
        &mut self,
        spec: &RenameDocumentSpec,
    ) -> Result<DocumentMutationResponse, WorkspaceCommandError> {
        rename_document(
            self.operations.store_mut(),
            &self.project_id,
            &self.main_branch_id,
            spec,
        )
    }

    pub fn reorder_document(
        &mut self,
        spec: &ReorderDocumentSpec,
    ) -> Result<DocumentMutationResponse, WorkspaceCommandError> {
        reorder_document(
            self.operations.store_mut(),
            &self.project_id,
            &self.main_branch_id,
            spec,
        )
    }

    pub fn change_document_depth(
        &mut self,
        spec: &ChangeDocumentDepthSpec,
    ) -> Result<DocumentMutationResponse, WorkspaceCommandError> {
        change_document_depth(
            self.operations.store_mut(),
            &self.project_id,
            &self.main_branch_id,
            spec,
        )
    }

    pub fn set_document_archived(
        &mut self,
        spec: &SetDocumentArchivedSpec,
    ) -> Result<DocumentMutationResponse, WorkspaceCommandError> {
        set_document_archived(
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

    pub fn knowledge_items(&self) -> Result<Vec<KnowledgeItem>, WorkspaceCommandError> {
        list_knowledge_items(self.operations.store(), &self.project_id)
    }

    pub fn create_knowledge_item(
        &mut self,
        spec: &CreateKnowledgeItemSpec,
    ) -> Result<KnowledgeItem, WorkspaceCommandError> {
        create_knowledge_item(self.operations.store_mut(), &self.project_id, spec)
    }

    pub fn set_knowledge_item_status(
        &mut self,
        spec: &SetKnowledgeItemStatusSpec,
    ) -> Result<KnowledgeItem, WorkspaceCommandError> {
        set_knowledge_item_status(self.operations.store_mut(), &self.project_id, spec)
    }

    pub fn knowledge_context(
        &self,
        spec: &KnowledgeContextSpec,
    ) -> Result<Vec<KnowledgeContextCandidate>, WorkspaceCommandError> {
        knowledge_context(self.operations.store(), &self.project_id, spec)
    }

    pub fn operation_context(
        &self,
        spec: &OperationContextSpec,
    ) -> Result<Vec<OperationContextCandidate>, WorkspaceCommandError> {
        collect_operation_context(self.operations.store(), &self.project_id, spec)
    }

    pub fn confirm_context_packet(
        &self,
        operation_context: &OperationContextSpec,
        operation_intent_id: &str,
        provider_locality: &str,
        payload: &serde_json::Value,
        request: &ModelExecutionRequest,
    ) -> Result<ConfirmedContextPacket, WorkspaceCommandError> {
        confirm_context_packet(
            self.operations.store(),
            &self.project_id,
            &ConfirmContextPacketInput {
                operation_context,
                operation_intent_id,
                provider_locality,
                payload,
                request,
            },
        )
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

    pub fn review_candidates(&self) -> Result<Vec<ReviewCandidateSummary>, WorkspaceCommandError> {
        list_review_candidates(self.operations.store(), &self.project_id)
    }

    pub fn review_candidate(
        &self,
        proposal_id: &str,
    ) -> Result<ReviewCandidateDetail, WorkspaceCommandError> {
        load_review_candidate(self.operations.store(), &self.project_id, proposal_id)
    }

    pub fn create_review_candidate_branch(
        &mut self,
        spec: &CreateReviewCandidateBranchSpec,
    ) -> Result<CreateReviewCandidateBranchResponse, WorkspaceCommandError> {
        create_review_candidate_branch(self.operations.store_mut(), &self.project_id, spec)
    }

    pub fn create_checkpoint(&mut self) -> Result<CheckpointSummary, WorkspaceCommandError> {
        create_checkpoint(self.operations.store_mut(), &self.project_id)
    }

    pub fn version_history(&self) -> Result<VersionHistory, WorkspaceCommandError> {
        load_version_history(self.operations.store(), &self.project_id)
    }

    pub fn timeline_events(&self) -> Result<TimelineEventsResponse, WorkspaceCommandError> {
        list_timeline_events(self.operations.store(), &self.project_id)
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

    pub fn compare_documents(
        &self,
        spec: &CompareDocumentsSpec,
    ) -> Result<optimizer_store::DocumentDiffResult, WorkspaceCommandError> {
        compare_documents(self.operations.store(), &self.project_id, spec)
    }

    /// v0.8.0 Stage 1 (FR-11): export the entire `.optimizer` project package
    /// as a single `.optimizer-backup` zip archive. The caller provides
    /// optional endpoint and recent-project metadata as JSON strings — the
    /// host core never touches the Secret Store.
    pub fn export_project_backup(
        &self,
        request: &ExportProjectBackupRequest,
    ) -> Result<ExportProjectBackupResponse, ProjectPackageError> {
        export_project_backup(
            self.operations.store(),
            self.root.as_path(),
            request,
        )
        .map_err(ProjectPackageError::Workspace)
    }

    /// v0.8.0 Stage 1 (FR-11): restore a `.optimizer-backup` archive into a
    /// new project package. This is an associated function because it creates
    /// a new project rather than operating on the current session.
    pub fn import_project_backup(
        request: &ImportProjectBackupRequest,
    ) -> Result<ImportProjectBackupResponse, ProjectPackageError> {
        import_project_backup(request).map_err(ProjectPackageError::Workspace)
    }

    /// Expose the project root path for backup operations.
    pub fn root_path(&self) -> &Path {
        self.root.as_path()
    }
}
