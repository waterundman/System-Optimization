use std::sync::{Arc, Mutex, MutexGuard};

use optimizer_host::{
    OperationAuditResponse, OperationCommandError, OperationCommandHost, PersistOperationResponse,
    PersistReviewResponse, SecretReference, SecretStore, SecretStoreError, SecretValue,
};
use serde::Serialize;
use tauri::{Runtime, State};

mod command_manifest;

pub use command_manifest::REGISTERED_COMMANDS;

#[derive(Clone)]
pub struct DesktopState {
    operations: Arc<Mutex<OperationCommandHost>>,
    secrets: Arc<dyn SecretStore>,
}

impl DesktopState {
    pub fn new(operations: OperationCommandHost, secrets: Arc<dyn SecretStore>) -> Self {
        Self {
            operations: Arc::new(Mutex::new(operations)),
            secrets,
        }
    }

    pub fn persist_operation_bundle(&self, input: &str) -> CommandResult<PersistOperationResponse> {
        self.operation_host()?
            .persist_operation_bundle_json(input)
            .map_err(CommandError::from)
    }

    pub fn append_review_event(&self, input: &str) -> CommandResult<PersistReviewResponse> {
        self.operation_host()?
            .append_review_event_json(input)
            .map_err(CommandError::from)
    }

    pub fn get_operation_audit(&self, run_id: &str) -> CommandResult<OperationAuditResponse> {
        self.operation_host()?
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

    fn operation_host(&self) -> CommandResult<MutexGuard<'_, OperationCommandHost>> {
        self.operations
            .lock()
            .map_err(|_| CommandError::state_unavailable())
    }
}

pub type CommandResult<T> = Result<T, CommandError>;

#[derive(Debug, Clone, PartialEq, Eq, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct CommandError {
    pub code: String,
    pub message: String,
}

impl CommandError {
    fn state_unavailable() -> Self {
        Self {
            code: "HOST_STATE_UNAVAILABLE".into(),
            message: "Desktop host state is unavailable".into(),
        }
    }

    fn task_failed() -> Self {
        Self {
            code: "HOST_TASK_FAILED".into(),
            message: "Desktop host task failed".into(),
        }
    }
}

impl From<OperationCommandError> for CommandError {
    fn from(error: OperationCommandError) -> Self {
        Self {
            code: error.code().into(),
            message: error.public_message(),
        }
    }
}

impl From<SecretStoreError> for CommandError {
    fn from(error: SecretStoreError) -> Self {
        Self {
            code: error.code().into(),
            message: error.to_string(),
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
            persist_operation_bundle,
            append_review_event,
            get_operation_audit,
            store_provider_secret,
            has_provider_secret,
            delete_provider_secret,
        ])
}

#[tauri::command]
async fn persist_operation_bundle(
    input: String,
    state: State<'_, DesktopState>,
) -> CommandResult<PersistOperationResponse> {
    let state = state.inner().clone();
    tauri::async_runtime::spawn_blocking(move || state.persist_operation_bundle(&input))
        .await
        .map_err(|_| CommandError::task_failed())?
}

#[tauri::command]
async fn append_review_event(
    input: String,
    state: State<'_, DesktopState>,
) -> CommandResult<PersistReviewResponse> {
    let state = state.inner().clone();
    tauri::async_runtime::spawn_blocking(move || state.append_review_event(&input))
        .await
        .map_err(|_| CommandError::task_failed())?
}

#[tauri::command]
async fn get_operation_audit(
    run_id: String,
    state: State<'_, DesktopState>,
) -> CommandResult<OperationAuditResponse> {
    let state = state.inner().clone();
    tauri::async_runtime::spawn_blocking(move || state.get_operation_audit(&run_id))
        .await
        .map_err(|_| CommandError::task_failed())?
}

#[tauri::command]
async fn store_provider_secret(
    reference: String,
    secret: String,
    state: State<'_, DesktopState>,
) -> CommandResult<SecretMutationResponse> {
    let state = state.inner().clone();
    tauri::async_runtime::spawn_blocking(move || state.store_provider_secret(reference, secret))
        .await
        .map_err(|_| CommandError::task_failed())?
}

#[tauri::command]
async fn has_provider_secret(
    reference: String,
    state: State<'_, DesktopState>,
) -> CommandResult<SecretStatusResponse> {
    let state = state.inner().clone();
    tauri::async_runtime::spawn_blocking(move || state.has_provider_secret(reference))
        .await
        .map_err(|_| CommandError::task_failed())?
}

#[tauri::command]
async fn delete_provider_secret(
    reference: String,
    state: State<'_, DesktopState>,
) -> CommandResult<SecretMutationResponse> {
    let state = state.inner().clone();
    tauri::async_runtime::spawn_blocking(move || state.delete_provider_secret(reference))
        .await
        .map_err(|_| CommandError::task_failed())?
}

#[cfg(test)]
mod tests {
    use std::fs;
    use std::path::Path;

    use optimizer_host::MemorySecretStore;
    use optimizer_store::{OptimizerStore, ProjectSeed, SeedBlock, SeedDocument};
    use serde_json::json;

    use super::*;

    fn state() -> (DesktopState, Arc<MemorySecretStore>) {
        let mut store = OptimizerStore::open_in_memory().unwrap();
        store
            .initialize_project(&ProjectSeed {
                project_id: "project-1".into(),
                title: "Desktop adapter test".into(),
                language: "zh-CN".into(),
                initial_commit_id: "commit-initial".into(),
                initial_root_hash: "sha256:root-initial".into(),
                main_branch_id: "branch-main".into(),
                documents: vec![SeedDocument {
                    id: "document-1".into(),
                    parent_id: None,
                    kind: "chapter".into(),
                    title: "Chapter 1".into(),
                    order_key: "a0".into(),
                }],
                blocks: vec![SeedBlock {
                    id: "block-1".into(),
                    document_id: "document-1".into(),
                    kind: "paragraph".into(),
                    order_key: "a0".into(),
                    content_json: r#"{"type":"paragraph","text":"station platform"}"#.into(),
                    plain_text: "station platform".into(),
                    content_hash: "sha256:block-initial".into(),
                    locked: false,
                }],
                created_at: "2026-07-14T00:00:00.000Z".into(),
            })
            .unwrap();
        let secrets = Arc::new(MemorySecretStore::default());
        (
            DesktopState::new(OperationCommandHost::new(store), secrets.clone()),
            secrets,
        )
    }

    fn fixture() -> String {
        let path = Path::new(env!("CARGO_MANIFEST_DIR"))
            .join("../../../packages/protocol/fixtures/operation-persistence-bundle.v1.json");
        fs::read_to_string(path).unwrap()
    }

    #[test]
    fn desktop_state_persists_and_reads_the_shared_operation_fixture() {
        let (state, _) = state();
        let persisted = state.persist_operation_bundle(&fixture()).unwrap();
        assert_eq!(persisted.run_id, "run-fixture-1");
        let audit = state.get_operation_audit("run-fixture-1").unwrap();
        assert_eq!(audit.run.state, "review");
        assert_eq!(audit.artifact.unwrap().id, "proposal-fixture-1");
    }

    #[test]
    fn command_errors_are_structured_and_do_not_echo_secret_values() {
        let (state, _) = state();
        let mut payload: serde_json::Value = serde_json::from_str(&fixture()).unwrap();
        payload["contextPacket"]["payload"]["apiKey"] = json!("secret-never-echoed");
        let error = state
            .persist_operation_bundle(&payload.to_string())
            .unwrap_err();
        assert_eq!(error.code, "SENSITIVE_FIELD_FORBIDDEN");
        assert!(!error.message.contains("secret-never-echoed"));
    }

    #[test]
    fn secret_commands_never_serialize_or_return_plaintext() {
        let (state, store) = state();
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
                "allow-operation-write",
                "allow-operation-audit-read",
                "allow-provider-secret-manage"
            ])
        );

        let config: serde_json::Value =
            serde_json::from_str(&fs::read_to_string(root.join("tauri.conf.json")).unwrap())
                .unwrap();
        assert_eq!(
            config["app"]["security"]["capabilities"],
            json!(["main-local"])
        );
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
