use std::sync::{Arc, Mutex, MutexGuard};

use optimizer_host::{
    ApplyReviewedProposalResponse, ApplyReviewedProposalSpec, CancelModelRequestResponse,
    CheckpointSummary, CreateDocumentResponse, CreateDocumentSpec, CreateStyleSampleSpec,
    ExportMarkdownResponse, ModelExecutionHost, ModelExecutionRequest, ModelExecutionSummary,
    ModelGatewayError, ModelProviderId, ModelStreamEvent, NewProjectSpec, OllamaModelList,
    OpenedProject, OperationAuditResponse, OperationCommandError, PersistOperationResponse,
    PersistReviewResponse, ProjectInfo, ProjectPackageError, ProjectWorkspace,
    RestoreCheckpointResponse, RestoreCheckpointSpec, SaveBlockResponse, SaveBlockSpec,
    SecretReference, SecretStore, SecretStoreError, SecretValue, SetStyleSampleStatusSpec,
    StyleSample, VersionHistory, WorkspaceCommandError,
};
use serde::{Deserialize, Serialize};
use tauri::{Runtime, State, ipc::Channel};

mod command_manifest;

pub use command_manifest::REGISTERED_COMMANDS;

#[derive(Clone)]
pub struct DesktopState {
    session: Arc<Mutex<Option<OpenedProject>>>,
    secrets: Arc<dyn SecretStore>,
    models: ModelExecutionHost,
}

impl DesktopState {
    pub fn new(secrets: Arc<dyn SecretStore>) -> Self {
        let models = ModelExecutionHost::new(secrets.clone());
        Self {
            session: Arc::new(Mutex::new(None)),
            secrets,
            models,
        }
    }

    pub fn create_project(
        &self,
        input: CreateProjectRequest,
    ) -> CommandResult<ProjectSessionResponse> {
        input.validate()?;
        let mut session = self.project_session()?;
        if session.is_some() {
            return Err(CommandError::project_already_open());
        }
        let project = OpenedProject::create(
            &input.parent_directory,
            &NewProjectSpec {
                folder_name: input.folder_name,
                title: input.title,
                language: input.language,
            },
        )
        .map_err(CommandError::from)?;
        let info = project.info().map_err(CommandError::from)?;
        *session = Some(project);
        Ok(ProjectSessionResponse::open(info))
    }

    pub fn open_project(&self, input: OpenProjectRequest) -> CommandResult<ProjectSessionResponse> {
        input.validate()?;
        let mut session = self.project_session()?;
        if session.is_some() {
            return Err(CommandError::project_already_open());
        }
        let project = OpenedProject::open(&input.project_directory).map_err(CommandError::from)?;
        let info = project.info().map_err(CommandError::from)?;
        *session = Some(project);
        Ok(ProjectSessionResponse::open(info))
    }

    pub fn close_project(&self) -> CommandResult<ProjectSessionResponse> {
        self.models.cancel_all();
        self.project_session()?.take();
        Ok(ProjectSessionResponse::closed())
    }

    pub fn get_project_session(&self) -> CommandResult<ProjectSessionResponse> {
        let session = self.project_session()?;
        match session.as_ref() {
            Some(project) => Ok(ProjectSessionResponse::open(
                project.info().map_err(CommandError::from)?,
            )),
            None => Ok(ProjectSessionResponse::closed()),
        }
    }

    pub fn get_project_workspace(&self) -> CommandResult<ProjectWorkspace> {
        self.current_project()?
            .workspace()
            .map_err(CommandError::from)
    }

    pub fn create_document(
        &self,
        input: CreateDocumentRequest,
    ) -> CommandResult<CreateDocumentResponse> {
        input.validate()?;
        let mut project = self.current_project()?;
        project
            .create_document(&CreateDocumentSpec {
                title: input.title,
                initial_text: input.initial_text,
            })
            .map_err(CommandError::from)
    }

    pub fn export_markdown(&self) -> CommandResult<ExportMarkdownResponse> {
        self.current_project()?
            .export_markdown()
            .map_err(CommandError::from)
    }

    pub fn list_style_samples(&self) -> CommandResult<Vec<StyleSample>> {
        self.current_project()?
            .style_samples()
            .map_err(CommandError::from)
    }

    pub fn create_style_sample(
        &self,
        input: CreateStyleSampleRequest,
    ) -> CommandResult<StyleSample> {
        input.validate()?;
        let mut project = self.current_project()?;
        project
            .create_style_sample(&CreateStyleSampleSpec {
                title: input.title,
                content: input.content,
                sensitivity: input.sensitivity,
            })
            .map_err(CommandError::from)
    }

    pub fn set_style_sample_status(
        &self,
        input: SetStyleSampleStatusRequest,
    ) -> CommandResult<StyleSample> {
        input.validate()?;
        let mut project = self.current_project()?;
        project
            .set_style_sample_status(&SetStyleSampleStatusSpec {
                id: input.id,
                expected_revision: input.expected_revision,
                status: input.status,
            })
            .map_err(CommandError::from)
    }

    pub fn save_block(&self, input: SaveBlockRequest) -> CommandResult<SaveBlockResponse> {
        input.validate()?;
        let mut project = self.current_project()?;
        project
            .save_block(&SaveBlockSpec {
                block_id: input.block_id,
                expected_revision: input.expected_revision,
                expected_hash: input.expected_hash,
                content: input.content,
                plain_text: input.plain_text,
            })
            .map_err(CommandError::from)
    }

    pub fn apply_reviewed_proposal(
        &self,
        input: ApplyReviewedProposalRequest,
    ) -> CommandResult<ApplyReviewedProposalResponse> {
        input.validate()?;
        let mut project = self.current_project()?;
        project
            .apply_reviewed_proposal(&ApplyReviewedProposalSpec {
                proposal_id: input.proposal_id,
                expected_review_revision: input.expected_review_revision,
            })
            .map_err(CommandError::from)
    }

    pub fn get_version_history(&self) -> CommandResult<VersionHistory> {
        self.current_project()?
            .version_history()
            .map_err(CommandError::from)
    }

    pub fn create_checkpoint(&self) -> CommandResult<CheckpointSummary> {
        let mut project = self.current_project()?;
        project.create_checkpoint().map_err(CommandError::from)
    }

    pub fn restore_checkpoint(
        &self,
        input: RestoreCheckpointRequest,
    ) -> CommandResult<RestoreCheckpointResponse> {
        input.validate()?;
        let mut project = self.current_project()?;
        project
            .restore_checkpoint(&RestoreCheckpointSpec {
                checkpoint_id: input.checkpoint_id,
            })
            .map_err(CommandError::from)
    }

    pub fn persist_operation_bundle(&self, input: &str) -> CommandResult<PersistOperationResponse> {
        let mut project = self.current_project()?;
        project
            .operations_mut()
            .persist_operation_bundle_json(input)
            .map_err(CommandError::from)
    }

    pub fn append_review_event(&self, input: &str) -> CommandResult<PersistReviewResponse> {
        let mut project = self.current_project()?;
        project
            .operations_mut()
            .append_review_event_json(input)
            .map_err(CommandError::from)
    }

    pub fn get_operation_audit(&self, run_id: &str) -> CommandResult<OperationAuditResponse> {
        self.current_project()?
            .operations()
            .get_operation_audit(run_id)
            .map_err(CommandError::from)
    }

    pub fn store_provider_secret(
        &self,
        reference: String,
        secret: String,
    ) -> CommandResult<SecretMutationResponse> {
        let reference = SecretReference::parse(reference).map_err(CommandError::from)?;
        let secret = SecretValue::new(secret).map_err(CommandError::from)?;
        self.secrets
            .put(&reference, secret)
            .map_err(CommandError::from)?;
        Ok(SecretMutationResponse {
            schema_version: 1,
            reference: reference.as_str().into(),
            changed: true,
            exists: true,
        })
    }

    pub fn has_provider_secret(&self, reference: String) -> CommandResult<SecretStatusResponse> {
        let reference = SecretReference::parse(reference).map_err(CommandError::from)?;
        let exists = self
            .secrets
            .contains(&reference)
            .map_err(CommandError::from)?;
        Ok(SecretStatusResponse {
            schema_version: 1,
            reference: reference.as_str().into(),
            exists,
        })
    }

    pub fn delete_provider_secret(
        &self,
        reference: String,
    ) -> CommandResult<SecretMutationResponse> {
        let reference = SecretReference::parse(reference).map_err(CommandError::from)?;
        let changed = self
            .secrets
            .delete(&reference)
            .map_err(CommandError::from)?;
        Ok(SecretMutationResponse {
            schema_version: 1,
            reference: reference.as_str().into(),
            changed,
            exists: false,
        })
    }

    pub fn execute_model_stream<F>(
        &self,
        input: ModelExecutionRequest,
        emit: F,
    ) -> CommandResult<ModelExecutionSummary>
    where
        F: FnMut(ModelStreamEvent) -> Result<(), ModelGatewayError>,
    {
        if self.project_session()?.is_none() {
            return Err(CommandError::no_project_open());
        }
        self.models
            .execute_stream(input, emit)
            .map_err(CommandError::from)
    }

    pub fn cancel_model_request(
        &self,
        request_id: String,
    ) -> CommandResult<CancelModelRequestResponse> {
        Ok(self.models.cancel(request_id))
    }

    pub fn list_ollama_models(&self) -> CommandResult<OllamaModelList> {
        self.models.list_ollama_models().map_err(CommandError::from)
    }

    fn project_session(&self) -> CommandResult<MutexGuard<'_, Option<OpenedProject>>> {
        self.session
            .lock()
            .map_err(|_| CommandError::state_unavailable())
    }

    fn current_project(&self) -> CommandResult<OpenedProjectGuard<'_>> {
        let guard = self.project_session()?;
        if guard.is_none() {
            return Err(CommandError::no_project_open());
        }
        Ok(OpenedProjectGuard(guard))
    }
}

struct OpenedProjectGuard<'a>(MutexGuard<'a, Option<OpenedProject>>);

impl OpenedProjectGuard<'_> {
    fn project(&self) -> &OpenedProject {
        self.0.as_ref().expect("open project guard invariant")
    }

    fn project_mut(&mut self) -> &mut OpenedProject {
        self.0.as_mut().expect("open project guard invariant")
    }
}

impl std::ops::Deref for OpenedProjectGuard<'_> {
    type Target = OpenedProject;

    fn deref(&self) -> &Self::Target {
        self.project()
    }
}

impl std::ops::DerefMut for OpenedProjectGuard<'_> {
    fn deref_mut(&mut self) -> &mut Self::Target {
        self.project_mut()
    }
}

pub type CommandResult<T> = Result<T, CommandError>;

#[derive(Debug, Clone, PartialEq, Eq, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct CommandError {
    pub code: String,
    pub message: String,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub details: Option<CommandErrorDetails>,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct CommandErrorDetails {
    #[serde(skip_serializing_if = "Option::is_none")]
    pub provider_id: Option<ModelProviderId>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub status: Option<u16>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub remote_code: Option<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub remote_request_id: Option<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub retry_after_ms: Option<u64>,
    pub retriable: bool,
}

impl CommandError {
    fn basic(code: impl Into<String>, message: impl Into<String>) -> Self {
        Self {
            code: code.into(),
            message: message.into(),
            details: None,
        }
    }

    fn state_unavailable() -> Self {
        Self::basic(
            "HOST_STATE_UNAVAILABLE",
            "Desktop host state is unavailable",
        )
    }

    fn task_failed() -> Self {
        Self::basic("HOST_TASK_FAILED", "Desktop host task failed")
    }

    fn no_project_open() -> Self {
        Self::basic(
            "NO_PROJECT_OPEN",
            "Open a project before using this command",
        )
    }

    fn project_already_open() -> Self {
        Self::basic(
            "PROJECT_ALREADY_OPEN",
            "Close the current project before opening another one",
        )
    }

    fn unsupported_request_schema(actual: u32) -> Self {
        Self::basic(
            "UNSUPPORTED_SCHEMA",
            format!("Unsupported project command schema version {actual}"),
        )
    }
}

impl From<OperationCommandError> for CommandError {
    fn from(error: OperationCommandError) -> Self {
        Self::basic(error.code(), error.public_message())
    }
}

impl From<ProjectPackageError> for CommandError {
    fn from(error: ProjectPackageError) -> Self {
        Self::basic(error.code(), error.public_message())
    }
}

impl From<SecretStoreError> for CommandError {
    fn from(error: SecretStoreError) -> Self {
        Self::basic(error.code(), error.to_string())
    }
}

impl From<WorkspaceCommandError> for CommandError {
    fn from(error: WorkspaceCommandError) -> Self {
        Self::basic(error.code(), error.public_message())
    }
}

impl From<ModelGatewayError> for CommandError {
    fn from(error: ModelGatewayError) -> Self {
        Self {
            code: error.code().into(),
            message: error.public_message(),
            details: Some(CommandErrorDetails {
                provider_id: error.provider_id(),
                status: error.status(),
                remote_code: error.remote_code().map(str::to_owned),
                remote_request_id: error.remote_request_id().map(str::to_owned),
                retry_after_ms: error.retry_after_ms(),
                retriable: error.retriable(),
            }),
        }
    }
}

#[derive(Debug, Clone, PartialEq, Eq, Deserialize)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
pub struct CreateProjectRequest {
    pub schema_version: u32,
    pub parent_directory: String,
    pub folder_name: String,
    pub title: String,
    pub language: String,
}

impl CreateProjectRequest {
    fn validate(&self) -> CommandResult<()> {
        if self.schema_version != 1 {
            return Err(CommandError::unsupported_request_schema(
                self.schema_version,
            ));
        }
        Ok(())
    }
}

#[derive(Debug, Clone, PartialEq, Eq, Deserialize)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
pub struct OpenProjectRequest {
    pub schema_version: u32,
    pub project_directory: String,
}

#[derive(Debug, Clone, PartialEq, Deserialize)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
pub struct SaveBlockRequest {
    pub schema_version: u32,
    pub block_id: String,
    pub expected_revision: i64,
    pub expected_hash: String,
    pub content: serde_json::Value,
    pub plain_text: String,
}

#[derive(Debug, Clone, PartialEq, Eq, Deserialize)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
pub struct CreateDocumentRequest {
    pub schema_version: u32,
    pub title: String,
    #[serde(default)]
    pub initial_text: String,
}

impl CreateDocumentRequest {
    fn validate(&self) -> CommandResult<()> {
        if self.schema_version != 1 {
            return Err(CommandError::unsupported_request_schema(
                self.schema_version,
            ));
        }
        Ok(())
    }
}

#[derive(Debug, Clone, PartialEq, Eq, Deserialize)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
pub struct CreateStyleSampleRequest {
    pub schema_version: u32,
    pub title: String,
    pub content: String,
    pub sensitivity: String,
}

impl CreateStyleSampleRequest {
    fn validate(&self) -> CommandResult<()> {
        if self.schema_version != 1 {
            return Err(CommandError::unsupported_request_schema(
                self.schema_version,
            ));
        }
        Ok(())
    }
}

#[derive(Debug, Clone, PartialEq, Eq, Deserialize)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
pub struct SetStyleSampleStatusRequest {
    pub schema_version: u32,
    pub id: String,
    pub expected_revision: i64,
    pub status: String,
}

impl SetStyleSampleStatusRequest {
    fn validate(&self) -> CommandResult<()> {
        if self.schema_version != 1 {
            return Err(CommandError::unsupported_request_schema(
                self.schema_version,
            ));
        }
        Ok(())
    }
}

impl SaveBlockRequest {
    fn validate(&self) -> CommandResult<()> {
        if self.schema_version != 1 {
            return Err(CommandError::unsupported_request_schema(
                self.schema_version,
            ));
        }
        Ok(())
    }
}

#[derive(Debug, Clone, PartialEq, Eq, Deserialize)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
pub struct ApplyReviewedProposalRequest {
    pub schema_version: u32,
    pub proposal_id: String,
    pub expected_review_revision: i64,
}

impl ApplyReviewedProposalRequest {
    fn validate(&self) -> CommandResult<()> {
        if self.schema_version != 1 {
            return Err(CommandError::unsupported_request_schema(
                self.schema_version,
            ));
        }
        Ok(())
    }
}

#[derive(Debug, Clone, PartialEq, Eq, Deserialize)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
pub struct RestoreCheckpointRequest {
    pub schema_version: u32,
    pub checkpoint_id: String,
}

impl RestoreCheckpointRequest {
    fn validate(&self) -> CommandResult<()> {
        if self.schema_version != 1 {
            return Err(CommandError::unsupported_request_schema(
                self.schema_version,
            ));
        }
        Ok(())
    }
}

impl OpenProjectRequest {
    fn validate(&self) -> CommandResult<()> {
        if self.schema_version != 1 {
            return Err(CommandError::unsupported_request_schema(
                self.schema_version,
            ));
        }
        Ok(())
    }
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct ProjectSessionResponse {
    pub schema_version: u32,
    pub is_open: bool,
    pub project: Option<ProjectInfo>,
}

impl ProjectSessionResponse {
    fn open(project: ProjectInfo) -> Self {
        Self {
            schema_version: 1,
            is_open: true,
            project: Some(project),
        }
    }

    fn closed() -> Self {
        Self {
            schema_version: 1,
            is_open: false,
            project: None,
        }
    }
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct SecretStatusResponse {
    pub schema_version: u32,
    pub reference: String,
    pub exists: bool,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct SecretMutationResponse {
    pub schema_version: u32,
    pub reference: String,
    pub changed: bool,
    pub exists: bool,
}

pub fn attach<R: Runtime>(builder: tauri::Builder<R>, state: DesktopState) -> tauri::Builder<R> {
    builder
        .manage(state)
        .invoke_handler(tauri::generate_handler![
            create_project,
            open_project,
            close_project,
            get_project_session,
            get_project_workspace,
            create_document,
            export_markdown,
            list_style_samples,
            create_style_sample,
            set_style_sample_status,
            save_block,
            apply_reviewed_proposal,
            get_version_history,
            create_checkpoint,
            restore_checkpoint,
            persist_operation_bundle,
            append_review_event,
            get_operation_audit,
            store_provider_secret,
            has_provider_secret,
            delete_provider_secret,
            execute_model_stream,
            cancel_model_request,
            list_ollama_models,
        ])
}

#[tauri::command]
async fn create_project(
    input: CreateProjectRequest,
    state: State<'_, DesktopState>,
) -> CommandResult<ProjectSessionResponse> {
    spawn_host_task(state, move |state| state.create_project(input)).await
}

#[tauri::command]
async fn open_project(
    input: OpenProjectRequest,
    state: State<'_, DesktopState>,
) -> CommandResult<ProjectSessionResponse> {
    spawn_host_task(state, move |state| state.open_project(input)).await
}

#[tauri::command]
async fn close_project(state: State<'_, DesktopState>) -> CommandResult<ProjectSessionResponse> {
    spawn_host_task(state, DesktopState::close_project).await
}

#[tauri::command]
async fn get_project_session(
    state: State<'_, DesktopState>,
) -> CommandResult<ProjectSessionResponse> {
    spawn_host_task(state, DesktopState::get_project_session).await
}

#[tauri::command]
async fn get_project_workspace(state: State<'_, DesktopState>) -> CommandResult<ProjectWorkspace> {
    spawn_host_task(state, DesktopState::get_project_workspace).await
}

#[tauri::command]
async fn create_document(
    input: CreateDocumentRequest,
    state: State<'_, DesktopState>,
) -> CommandResult<CreateDocumentResponse> {
    spawn_host_task(state, move |state| state.create_document(input)).await
}

#[tauri::command]
async fn export_markdown(state: State<'_, DesktopState>) -> CommandResult<ExportMarkdownResponse> {
    spawn_host_task(state, DesktopState::export_markdown).await
}

#[tauri::command]
async fn list_style_samples(state: State<'_, DesktopState>) -> CommandResult<Vec<StyleSample>> {
    spawn_host_task(state, DesktopState::list_style_samples).await
}

#[tauri::command]
async fn create_style_sample(
    input: CreateStyleSampleRequest,
    state: State<'_, DesktopState>,
) -> CommandResult<StyleSample> {
    spawn_host_task(state, move |state| state.create_style_sample(input)).await
}

#[tauri::command]
async fn set_style_sample_status(
    input: SetStyleSampleStatusRequest,
    state: State<'_, DesktopState>,
) -> CommandResult<StyleSample> {
    spawn_host_task(state, move |state| state.set_style_sample_status(input)).await
}

#[tauri::command]
async fn save_block(
    input: SaveBlockRequest,
    state: State<'_, DesktopState>,
) -> CommandResult<SaveBlockResponse> {
    spawn_host_task(state, move |state| state.save_block(input)).await
}

#[tauri::command]
async fn apply_reviewed_proposal(
    input: ApplyReviewedProposalRequest,
    state: State<'_, DesktopState>,
) -> CommandResult<ApplyReviewedProposalResponse> {
    spawn_host_task(state, move |state| state.apply_reviewed_proposal(input)).await
}

#[tauri::command]
async fn get_version_history(state: State<'_, DesktopState>) -> CommandResult<VersionHistory> {
    spawn_host_task(state, DesktopState::get_version_history).await
}

#[tauri::command]
async fn create_checkpoint(state: State<'_, DesktopState>) -> CommandResult<CheckpointSummary> {
    spawn_host_task(state, DesktopState::create_checkpoint).await
}

#[tauri::command]
async fn restore_checkpoint(
    input: RestoreCheckpointRequest,
    state: State<'_, DesktopState>,
) -> CommandResult<RestoreCheckpointResponse> {
    spawn_host_task(state, move |state| state.restore_checkpoint(input)).await
}

#[tauri::command]
async fn persist_operation_bundle(
    input: String,
    state: State<'_, DesktopState>,
) -> CommandResult<PersistOperationResponse> {
    spawn_host_task(state, move |state| state.persist_operation_bundle(&input)).await
}

#[tauri::command]
async fn append_review_event(
    input: String,
    state: State<'_, DesktopState>,
) -> CommandResult<PersistReviewResponse> {
    spawn_host_task(state, move |state| state.append_review_event(&input)).await
}

#[tauri::command]
async fn get_operation_audit(
    run_id: String,
    state: State<'_, DesktopState>,
) -> CommandResult<OperationAuditResponse> {
    spawn_host_task(state, move |state| state.get_operation_audit(&run_id)).await
}

#[tauri::command]
async fn store_provider_secret(
    reference: String,
    secret: String,
    state: State<'_, DesktopState>,
) -> CommandResult<SecretMutationResponse> {
    spawn_host_task(state, move |state| {
        state.store_provider_secret(reference, secret)
    })
    .await
}

#[tauri::command]
async fn has_provider_secret(
    reference: String,
    state: State<'_, DesktopState>,
) -> CommandResult<SecretStatusResponse> {
    spawn_host_task(state, move |state| state.has_provider_secret(reference)).await
}

#[tauri::command]
async fn delete_provider_secret(
    reference: String,
    state: State<'_, DesktopState>,
) -> CommandResult<SecretMutationResponse> {
    spawn_host_task(state, move |state| state.delete_provider_secret(reference)).await
}

#[tauri::command]
async fn execute_model_stream(
    input: ModelExecutionRequest,
    on_event: Channel<ModelStreamEvent>,
    state: State<'_, DesktopState>,
) -> CommandResult<ModelExecutionSummary> {
    spawn_host_task(state, move |state| {
        state.execute_model_stream(input, |event| {
            on_event
                .send(event)
                .map_err(|_| ModelGatewayError::event_delivery_failed())
        })
    })
    .await
}

#[tauri::command]
async fn cancel_model_request(
    request_id: String,
    state: State<'_, DesktopState>,
) -> CommandResult<CancelModelRequestResponse> {
    spawn_host_task(state, move |state| state.cancel_model_request(request_id)).await
}

#[tauri::command]
async fn list_ollama_models(state: State<'_, DesktopState>) -> CommandResult<OllamaModelList> {
    spawn_host_task(state, DesktopState::list_ollama_models).await
}

async fn spawn_host_task<T, F>(state: State<'_, DesktopState>, task: F) -> CommandResult<T>
where
    T: Send + 'static,
    F: FnOnce(&DesktopState) -> CommandResult<T> + Send + 'static,
{
    let state = state.inner().clone();
    tauri::async_runtime::spawn_blocking(move || task(&state))
        .await
        .map_err(|_| CommandError::task_failed())?
}

#[cfg(test)]
mod tests {
    use std::fs;
    use std::path::{Path, PathBuf};
    use std::sync::atomic::{AtomicU64, Ordering};
    use std::time::{SystemTime, UNIX_EPOCH};

    use optimizer_host::MemorySecretStore;
    use serde_json::json;

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
                "optimizer-desktop-session-{}-{nonce}-{}",
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

    fn state() -> (DesktopState, Arc<MemorySecretStore>, TempParent) {
        let secrets = Arc::new(MemorySecretStore::default());
        (
            DesktopState::new(secrets.clone()),
            secrets,
            TempParent::new(),
        )
    }

    fn create_request(parent: &TempParent) -> CreateProjectRequest {
        CreateProjectRequest {
            schema_version: 1,
            parent_directory: parent.0.to_string_lossy().into_owned(),
            folder_name: "DesktopTest.optimizer".into(),
            title: "Desktop Test".into(),
            language: "zh-CN".into(),
        }
    }

    fn open_test_project(state: &DesktopState, parent: &TempParent) -> ProjectInfo {
        state
            .create_project(create_request(parent))
            .unwrap()
            .project
            .unwrap()
    }

    fn fixture(info: &ProjectInfo) -> String {
        let path = Path::new(env!("CARGO_MANIFEST_DIR"))
            .join("../../../packages/protocol/fixtures/operation-persistence-bundle.v1.json");
        let mut fixture: serde_json::Value =
            serde_json::from_slice(&fs::read(path).unwrap()).unwrap();
        fixture["run"]["projectId"] = json!(info.project_id);
        fixture["run"]["baseCommitId"] = json!(info.head_commit_id);
        fixture["contextPacket"]["projectId"] = json!(info.project_id);
        fixture["contextPacket"]["baseCommitId"] = json!(info.head_commit_id);
        fixture["contextPacket"]["payload"]["projectId"] = json!(info.project_id);
        fixture["contextPacket"]["payload"]["baseCommitId"] = json!(info.head_commit_id);
        fixture.to_string()
    }

    #[test]
    fn creates_closes_and_reopens_a_project_session() {
        let (state, _, parent) = state();
        assert!(!state.get_project_session().unwrap().is_open);
        let created = state.create_project(create_request(&parent)).unwrap();
        let info = created.project.unwrap();
        assert!(Path::new(&info.directory).is_dir());
        assert!(state.get_project_session().unwrap().is_open);
        assert_eq!(
            state
                .create_project(create_request(&parent))
                .unwrap_err()
                .code,
            "PROJECT_ALREADY_OPEN"
        );
        assert!(!state.close_project().unwrap().is_open);
        let reopened = state
            .open_project(OpenProjectRequest {
                schema_version: 1,
                project_directory: info.directory,
            })
            .unwrap();
        assert_eq!(reopened.project.unwrap().project_id, info.project_id);
    }

    #[test]
    fn operation_commands_require_and_use_the_current_project_session() {
        let (state, _, parent) = state();
        assert_eq!(
            state.persist_operation_bundle("{}").unwrap_err().code,
            "NO_PROJECT_OPEN"
        );
        let info = open_test_project(&state, &parent);
        let persisted = state.persist_operation_bundle(&fixture(&info)).unwrap();
        assert_eq!(persisted.run_id, "run-fixture-1");
        let audit = state.get_operation_audit("run-fixture-1").unwrap();
        assert_eq!(audit.run.state, "review");
        assert_eq!(audit.run.project_id, info.project_id);
    }

    #[test]
    fn workspace_commands_load_and_optimistically_save_the_current_project() {
        let (state, _, parent) = state();
        assert_eq!(
            state.get_project_workspace().unwrap_err().code,
            "NO_PROJECT_OPEN"
        );
        open_test_project(&state, &parent);
        let workspace = state.get_project_workspace().unwrap();
        assert_eq!(workspace.documents.len(), 1);
        assert_eq!(workspace.blocks.len(), 1);
        let block = workspace.blocks[0].clone();
        let request = SaveBlockRequest {
            schema_version: 1,
            block_id: block.id.clone(),
            expected_revision: block.revision,
            expected_hash: block.content_hash.clone(),
            content: json!({
                "type": "paragraph",
                "content": [{ "type": "text", "text": "桌面自动保存" }]
            }),
            plain_text: "桌面自动保存".into(),
        };
        let saved = state.save_block(request.clone()).unwrap();
        assert_eq!(saved.project_revision, 1);
        assert_eq!(saved.block.revision, 1);
        assert_eq!(saved.block.plain_text, "桌面自动保存");
        assert_eq!(state.save_block(request).unwrap_err().code, "CONFLICT");
        assert_eq!(
            state
                .save_block(SaveBlockRequest {
                    schema_version: 99,
                    block_id: block.id,
                    expected_revision: 1,
                    expected_hash: saved.block.content_hash.clone(),
                    content: json!({}),
                    plain_text: String::new(),
                })
                .unwrap_err()
                .code,
            "UNSUPPORTED_SCHEMA"
        );

        let checkpoint = state.create_checkpoint().unwrap();
        let history = state.get_version_history().unwrap();
        assert_eq!(history.commits.len(), 2);
        assert_eq!(history.checkpoints, vec![checkpoint.clone()]);
        assert_eq!(checkpoint.commit_id, saved.commit_id);

        let second = state
            .save_block(SaveBlockRequest {
                schema_version: 1,
                block_id: saved.block.id.clone(),
                expected_revision: saved.block.revision,
                expected_hash: saved.block.content_hash.clone(),
                content: json!({
                    "type": "paragraph",
                    "content": [{ "type": "text", "text": "稍后改写" }]
                }),
                plain_text: "稍后改写".into(),
            })
            .unwrap();
        assert_eq!(second.project_revision, 2);
        let restored = state
            .restore_checkpoint(RestoreCheckpointRequest {
                schema_version: 1,
                checkpoint_id: checkpoint.id,
            })
            .unwrap();
        assert_eq!(restored.changed_blocks, 1);
        assert_eq!(restored.workspace.revision, 3);
        assert_eq!(restored.workspace.documents[0].revision, 3);
        assert_eq!(restored.workspace.blocks[0].plain_text, "桌面自动保存");
        assert_eq!(state.get_version_history().unwrap().commits.len(), 4);
    }

    #[test]
    fn style_library_is_project_scoped_and_uses_optimistic_status_updates() {
        let (state, _, parent) = state();
        assert_eq!(
            state.list_style_samples().unwrap_err().code,
            "NO_PROJECT_OPEN"
        );
        open_test_project(&state, &parent);
        let created = state
            .create_style_sample(CreateStyleSampleRequest {
                schema_version: 1,
                title: "克制短句".into(),
                content: "风停了。灯还亮着。".into(),
                sensitivity: "local_sensitive".into(),
            })
            .unwrap();
        assert_eq!(created.status, "canonical");
        assert_eq!(state.list_style_samples().unwrap(), vec![created.clone()]);

        let archived = state
            .set_style_sample_status(SetStyleSampleStatusRequest {
                schema_version: 1,
                id: created.id.clone(),
                expected_revision: 0,
                status: "archived".into(),
            })
            .unwrap();
        assert_eq!(archived.revision, 1);
        assert_eq!(archived.status, "archived");
        assert_eq!(
            state
                .set_style_sample_status(SetStyleSampleStatusRequest {
                    schema_version: 1,
                    id: created.id,
                    expected_revision: 0,
                    status: "canonical".into(),
                })
                .unwrap_err()
                .code,
            "CONFLICT"
        );
    }

    #[test]
    fn creates_a_versioned_chapter_through_the_current_session() {
        let (state, _, parent) = state();
        let info = open_test_project(&state, &parent);
        let created = state
            .create_document(CreateDocumentRequest {
                schema_version: 1,
                title: "第二章".into(),
                initial_text: "雾从海面升起。".into(),
            })
            .unwrap();
        assert_eq!(created.document.title, "第二章");
        assert_eq!(created.block.plain_text, "雾从海面升起。");
        assert_eq!(created.project_revision, 1);
        let workspace = state.get_project_workspace().unwrap();
        assert_eq!(workspace.documents.len(), 2);
        assert_eq!(workspace.blocks.len(), 2);
        assert_eq!(workspace.head_commit_id, created.commit_id);
        assert!(
            state
                .get_version_history()
                .unwrap()
                .commits
                .iter()
                .any(|commit| commit.reason == "document_create")
        );
        let exported = state.export_markdown().unwrap();
        assert!(Path::new(&exported.path).starts_with(&info.directory));
        let markdown = fs::read_to_string(&exported.path).unwrap();
        assert!(markdown.contains("# Desktop Test"));
        assert!(markdown.contains("## 第二章"));
        assert!(markdown.contains("雾从海面升起。"));
    }

    #[test]
    fn command_errors_are_structured_and_do_not_echo_secret_values() {
        let (state, _, parent) = state();
        let info = open_test_project(&state, &parent);
        let mut payload: serde_json::Value = serde_json::from_str(&fixture(&info)).unwrap();
        payload["contextPacket"]["payload"]["apiKey"] = json!("secret-never-echoed");
        let error = state
            .persist_operation_bundle(&payload.to_string())
            .unwrap_err();
        assert_eq!(error.code, "SENSITIVE_FIELD_FORBIDDEN");
        assert!(!error.message.contains("secret-never-echoed"));
    }

    #[test]
    fn secret_commands_never_serialize_or_return_plaintext() {
        let (state, store, _) = state();
        let reference = "secret://providers/deepseek/default".to_string();
        let stored = state
            .store_provider_secret(reference.clone(), "desktop-test-key".into())
            .unwrap();
        assert!(stored.exists);
        assert!(
            !serde_json::to_string(&stored)
                .unwrap()
                .contains("desktop-test-key")
        );
        assert!(state.has_provider_secret(reference.clone()).unwrap().exists);

        let internal_reference = SecretReference::parse(reference.clone()).unwrap();
        assert_eq!(
            store
                .resolve(&internal_reference)
                .unwrap()
                .unwrap()
                .expose_secret(),
            "desktop-test-key"
        );
        let deleted = state.delete_provider_secret(reference.clone()).unwrap();
        assert!(deleted.changed);
        assert!(!deleted.exists);
        assert!(!state.has_provider_secret(reference).unwrap().exists);
    }

    #[test]
    fn model_execution_requires_a_project_and_reports_safe_provider_details() {
        let (state, _, parent) = state();
        let input: ModelExecutionRequest = serde_json::from_value(json!({
            "schemaVersion": 1,
            "requestId": "desktop-deepseek-test",
            "configuration": {
                "schemaVersion": 1,
                "id": "provider-deepseek-default",
                "providerId": "deepseek",
                "enabled": true,
                "defaultModel": "deepseek-chat",
                "credentialRef": "secret://providers/deepseek/default",
                "qwen": null,
                "defaultTimeoutMs": 60_000,
                "maxRequestBytes": 16 * 1024 * 1024,
                "updatedAt": "2026-07-15T00:00:00Z"
            },
            "request": {
                "model": null,
                "messages": [{
                    "role": "user",
                    "content": "优化这段文字",
                    "name": null,
                    "reasoningContent": null,
                    "toolCallId": null,
                    "toolCalls": null
                }],
                "maxOutputTokens": 512,
                "temperature": null,
                "topP": null,
                "stop": null,
                "responseFormat": null,
                "reasoning": null,
                "tools": null,
                "toolChoice": null
            }
        }))
        .unwrap();

        assert_eq!(
            state
                .execute_model_stream(input.clone(), |_| Ok(()))
                .unwrap_err()
                .code,
            "NO_PROJECT_OPEN"
        );
        open_test_project(&state, &parent);
        let error = state.execute_model_stream(input, |_| Ok(())).unwrap_err();
        assert_eq!(error.code, "PROVIDER_CREDENTIAL_MISSING");
        assert_eq!(
            error.details.as_ref().unwrap().provider_id,
            Some(ModelProviderId::Deepseek)
        );
        assert!(!error.details.as_ref().unwrap().retriable);
        let serialized = serde_json::to_string(&error).unwrap();
        assert!(!serialized.contains("apiKey"));
        assert!(!serialized.contains("Authorization"));

        let cancelled = state
            .cancel_model_request("not-active".to_string())
            .unwrap();
        assert_eq!(cancelled.request_id, "not-active");
        assert!(!cancelled.cancelled);
    }

    #[test]
    fn tauri_security_configuration_is_local_and_covers_every_registered_command() {
        let root = Path::new(env!("CARGO_MANIFEST_DIR"));
        let capability: serde_json::Value = serde_json::from_str(
            &fs::read_to_string(root.join("capabilities/main-local.json")).unwrap(),
        )
        .unwrap();
        assert_eq!(capability["windows"], json!(["main"]));
        assert!(capability.get("remote").is_none());
        assert_eq!(
            capability["permissions"],
            json!([
                "allow-project-session",
                "allow-workspace-read",
                "allow-style-library",
                "allow-workspace-write",
                "allow-project-export",
                "allow-version-read",
                "allow-version-write",
                "allow-operation-write",
                "allow-operation-audit-read",
                "allow-provider-secret-manage",
                "allow-model-execution"
            ])
        );

        let config: serde_json::Value =
            serde_json::from_str(&fs::read_to_string(root.join("tauri.conf.json")).unwrap())
                .unwrap();
        assert_eq!(
            config["app"]["security"]["capabilities"],
            json!(["main-local"])
        );
        assert_eq!(config["app"]["windows"][0]["create"], json!(false));
        assert!(config["app"]["security"]["csp"].as_str().is_some());

        let permissions = fs::read_to_string(root.join("permissions/optimizer.toml")).unwrap();
        for command in REGISTERED_COMMANDS {
            assert!(
                permissions.contains(command),
                "permission missing for {command}"
            );
        }
        assert!(!permissions.contains("resolve_provider_secret"));
        assert!(!permissions.contains("read_provider_secret"));
    }
}
