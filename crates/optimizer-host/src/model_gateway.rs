use std::collections::{BTreeMap, HashMap};
use std::fmt;
#[cfg(windows)]
use std::sync::atomic::AtomicUsize;
use std::sync::atomic::{AtomicU8, Ordering};
use std::sync::{Arc, Mutex, mpsc};
use std::time::Duration;

use serde::{Deserialize, Serialize};
use serde_json::{Map, Value, json};
use time::OffsetDateTime;
use time::format_description::well_known::{Rfc2822, Rfc3339};
use uuid::Uuid;

use crate::confirmed_context::ConfirmedContextPacket;
use crate::{SecretReference, SecretStore, SecretStoreError, SecretValue};

const REQUEST_SCHEMA_VERSION: u32 = 1;
const MAX_TIMEOUT_MS: u32 = 3_600_000;
const MAX_REQUEST_BYTES: u32 = 67_108_864;
const MAX_RESPONSE_BYTES: usize = 67_108_864;
const MAX_ERROR_BYTES: usize = 65_536;
const MAX_SSE_EVENT_BYTES: usize = 4 * 1024 * 1024;
const MAX_OUTPUT_BYTES: usize = 32 * 1024 * 1024;
const OLLAMA_DISCOVERY_TIMEOUT_MS: u32 = 3_000;
const OLLAMA_DISCOVERY_MAX_BYTES: usize = 1024 * 1024;
const MODEL_AUTHORIZATION_TTL_SECONDS: i64 = 120;
const MAX_PENDING_MODEL_AUTHORIZATIONS: usize = 32;
const CONTROL_ACTIVE: u8 = 0;
const CONTROL_CANCELLED: u8 = 1;
const CONTROL_TIMED_OUT: u8 = 2;
const CONTROL_COMPLETE: u8 = 3;

#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, Serialize, Deserialize)]
#[serde(rename_all = "lowercase")]
pub enum ModelProviderId {
    Deepseek,
    Qwen,
    Kimi,
    Minimax,
    Ollama,
}

impl ModelProviderId {
    pub fn as_str(self) -> &'static str {
        match self {
            Self::Deepseek => "deepseek",
            Self::Qwen => "qwen",
            Self::Kimi => "kimi",
            Self::Minimax => "minimax",
            Self::Ollama => "ollama",
        }
    }

    fn label(self) -> &'static str {
        match self {
            Self::Deepseek => "DeepSeek",
            Self::Qwen => "Qwen",
            Self::Kimi => "Kimi",
            Self::Minimax => "MiniMax",
            Self::Ollama => "Ollama",
        }
    }

    fn stream_mode(self) -> StreamContentMode {
        match self {
            Self::Minimax => StreamContentMode::Cumulative,
            _ => StreamContentMode::Delta,
        }
    }
}

impl fmt::Display for ModelProviderId {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        formatter.write_str(self.as_str())
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "lowercase")]
pub enum QwenDeploymentRegion {
    China,
    Singapore,
    Us,
    Germany,
    Japan,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
pub struct QwenProviderConfiguration {
    pub region: QwenDeploymentRegion,
    pub workspace_id: Option<String>,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
pub struct ModelProviderConfiguration {
    pub schema_version: u32,
    pub id: String,
    pub provider_id: ModelProviderId,
    pub enabled: bool,
    pub default_model: String,
    pub credential_ref: String,
    pub qwen: Option<QwenProviderConfiguration>,
    pub default_timeout_ms: u32,
    pub max_request_bytes: u32,
    pub updated_at: String,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "lowercase")]
pub enum ModelMessageRole {
    System,
    User,
    Assistant,
    Tool,
}

impl ModelMessageRole {
    fn as_str(self) -> &'static str {
        match self {
            Self::System => "system",
            Self::User => "user",
            Self::Assistant => "assistant",
            Self::Tool => "tool",
        }
    }
}

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
pub struct ModelToolCall {
    pub id: String,
    #[serde(rename = "type")]
    pub kind: String,
    pub function: ModelToolCallFunction,
}

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
pub struct ModelToolCallFunction {
    pub name: String,
    pub arguments: String,
}

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
pub struct ModelMessage {
    pub role: ModelMessageRole,
    pub content: String,
    pub name: Option<String>,
    pub reasoning_content: Option<String>,
    pub tool_call_id: Option<String>,
    pub tool_calls: Option<Vec<ModelToolCall>>,
}

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
pub struct ModelToolDefinition {
    #[serde(rename = "type")]
    pub kind: String,
    pub function: ModelToolDefinitionFunction,
}

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
pub struct ModelToolDefinitionFunction {
    pub name: String,
    pub description: Option<String>,
    pub parameters: Option<Value>,
    pub strict: Option<bool>,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "lowercase")]
pub enum ReasoningMode {
    Enabled,
    Disabled,
    Adaptive,
}

impl ReasoningMode {
    fn as_str(self) -> &'static str {
        match self {
            Self::Enabled => "enabled",
            Self::Disabled => "disabled",
            Self::Adaptive => "adaptive",
        }
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "lowercase")]
pub enum ReasoningEffort {
    Low,
    Medium,
    High,
    Max,
}

impl ReasoningEffort {
    fn as_str(self) -> &'static str {
        match self {
            Self::Low => "low",
            Self::Medium => "medium",
            Self::High => "high",
            Self::Max => "max",
        }
    }
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
pub struct ReasoningOptions {
    pub mode: ReasoningMode,
    pub effort: Option<ReasoningEffort>,
    pub preserve: Option<bool>,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum ModelResponseFormat {
    Text,
    JsonObject,
}

impl ModelResponseFormat {
    fn as_str(self) -> &'static str {
        match self {
            Self::Text => "text",
            Self::JsonObject => "json_object",
        }
    }
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "lowercase")]
pub enum ModelToolChoice {
    None,
    Auto,
    Required,
}

impl ModelToolChoice {
    fn as_str(&self) -> &'static str {
        match self {
            Self::None => "none",
            Self::Auto => "auto",
            Self::Required => "required",
        }
    }
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(untagged)]
pub enum ModelStopSequences {
    One(String),
    Many(Vec<String>),
}

impl ModelStopSequences {
    fn values(&self) -> Vec<&str> {
        match self {
            Self::One(value) => vec![value.as_str()],
            Self::Many(values) => values.iter().map(String::as_str).collect(),
        }
    }
}

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
pub struct ModelRequest {
    pub model: Option<String>,
    pub messages: Vec<ModelMessage>,
    pub max_output_tokens: Option<u32>,
    pub temperature: Option<f64>,
    pub top_p: Option<f64>,
    pub stop: Option<ModelStopSequences>,
    pub response_format: Option<ModelResponseFormat>,
    pub reasoning: Option<ReasoningOptions>,
    pub tools: Option<Vec<ModelToolDefinition>>,
    pub tool_choice: Option<ModelToolChoice>,
}

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
pub struct ModelExecutionRequest {
    pub schema_version: u32,
    pub request_id: String,
    pub configuration: ModelProviderConfiguration,
    pub request: ModelRequest,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct ModelAuthorizationScope {
    pub project_id: String,
    pub base_commit_id: String,
    pub operation_intent_id: String,
    pub confirmed_context: ConfirmedContextPacket,
    pub target_block_id: String,
    pub target_block_revision: i64,
    pub target_block_hash: String,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct ModelRequestAuthorization {
    pub schema_version: u32,
    pub authorization_id: String,
    pub request_id: String,
    pub provider_id: ModelProviderId,
    pub expires_at: String,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize)]
#[serde(rename_all = "snake_case")]
pub enum ModelFinishReason {
    Stop,
    Length,
    ContentFilter,
    ToolCalls,
    InsufficientSystemResource,
    Unknown,
}

impl ModelFinishReason {
    fn from_value(value: Option<&Value>) -> Self {
        match value.and_then(Value::as_str) {
            Some("stop") => Self::Stop,
            Some("length") => Self::Length,
            Some("content_filter") => Self::ContentFilter,
            Some("tool_calls") => Self::ToolCalls,
            Some("insufficient_system_resource") => Self::InsufficientSystemResource,
            _ => Self::Unknown,
        }
    }
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct ModelUsage {
    pub input_tokens: u64,
    pub output_tokens: u64,
    pub total_tokens: u64,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub cached_input_tokens: Option<u64>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub reasoning_tokens: Option<u64>,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize)]
#[serde(tag = "type", rename_all = "snake_case")]
pub enum ModelStreamEvent {
    Start {
        #[serde(rename = "requestId")]
        request_id: String,
        id: String,
        #[serde(rename = "providerId")]
        provider_id: ModelProviderId,
        model: String,
    },
    TextDelta {
        text: String,
    },
    ReasoningDelta {
        text: String,
    },
    ToolCallDelta {
        index: u64,
        #[serde(skip_serializing_if = "Option::is_none")]
        id: Option<String>,
        #[serde(skip_serializing_if = "Option::is_none")]
        name: Option<String>,
        #[serde(skip_serializing_if = "Option::is_none")]
        arguments: Option<String>,
    },
    Usage {
        usage: ModelUsage,
    },
    Finish {
        reason: ModelFinishReason,
    },
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct ModelExecutionSummary {
    pub schema_version: u32,
    pub request_id: String,
    pub response_id: String,
    pub provider_id: ModelProviderId,
    pub model: String,
    pub content: String,
    pub reasoning_content: String,
    pub finish_reason: ModelFinishReason,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub usage: Option<ModelUsage>,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct OllamaModelInfo {
    pub id: String,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub created: Option<u64>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub owned_by: Option<String>,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct OllamaModelList {
    pub schema_version: u32,
    pub endpoint: String,
    pub models: Vec<OllamaModelInfo>,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct CancelModelRequestResponse {
    pub schema_version: u32,
    pub request_id: String,
    pub cancelled: bool,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct ModelGatewayError {
    code: &'static str,
    message: String,
    provider_id: Option<ModelProviderId>,
    status: Option<u16>,
    remote_code: Option<String>,
    remote_request_id: Option<String>,
    retry_after_ms: Option<u64>,
    retriable: bool,
}

impl ModelGatewayError {
    fn new(code: &'static str, message: impl Into<String>) -> Self {
        Self {
            code,
            message: message.into(),
            provider_id: None,
            status: None,
            remote_code: None,
            remote_request_id: None,
            retry_after_ms: None,
            retriable: false,
        }
    }

    pub fn transport(message: impl Into<String>, retriable: bool) -> Self {
        let mut error = Self::new("PROVIDER_NETWORK", message);
        error.retriable = retriable;
        error
    }

    pub fn event_delivery_failed() -> Self {
        Self::new(
            "MODEL_EVENT_DELIVERY_FAILED",
            "model stream receiver is no longer available",
        )
    }

    pub fn code(&self) -> &'static str {
        self.code
    }

    pub fn public_message(&self) -> String {
        self.message.clone()
    }

    pub fn provider_id(&self) -> Option<ModelProviderId> {
        self.provider_id
    }

    pub fn status(&self) -> Option<u16> {
        self.status
    }

    pub fn remote_code(&self) -> Option<&str> {
        self.remote_code.as_deref()
    }

    pub fn remote_request_id(&self) -> Option<&str> {
        self.remote_request_id.as_deref()
    }

    pub fn retry_after_ms(&self) -> Option<u64> {
        self.retry_after_ms
    }

    pub fn retriable(&self) -> bool {
        self.retriable
    }

    fn for_provider(mut self, provider_id: ModelProviderId) -> Self {
        self.provider_id = Some(provider_id);
        self
    }
}

impl fmt::Display for ModelGatewayError {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        formatter.write_str(&self.message)
    }
}

impl std::error::Error for ModelGatewayError {}

impl From<SecretStoreError> for ModelGatewayError {
    fn from(error: SecretStoreError) -> Self {
        Self::new(error.code(), error.to_string())
    }
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct ModelHttpResponseHead {
    pub status: u16,
    pub headers: BTreeMap<String, String>,
}

impl ModelHttpResponseHead {
    pub fn header(&self, name: &str) -> Option<&str> {
        self.headers
            .get(&name.to_ascii_lowercase())
            .map(String::as_str)
    }
}

pub trait ModelResponseSink {
    fn begin(&mut self, head: ModelHttpResponseHead) -> Result<(), ModelGatewayError>;
    fn chunk(&mut self, chunk: &[u8]) -> Result<(), ModelGatewayError>;
}

pub trait ModelTransport: Send + Sync {
    fn execute(
        &self,
        request: &PreparedModelRequest,
        secret: Option<&SecretValue>,
        cancellation: &ModelCancellation,
        sink: &mut dyn ModelResponseSink,
    ) -> Result<(), ModelGatewayError>;
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct PreparedModelRequest {
    request_id: String,
    provider_id: ModelProviderId,
    host: String,
    path: String,
    port: u16,
    use_tls: bool,
    method: ModelHttpMethod,
    body: Vec<u8>,
    timeout_ms: u32,
    max_response_bytes: usize,
}

impl PreparedModelRequest {
    pub fn request_id(&self) -> &str {
        &self.request_id
    }

    pub fn provider_id(&self) -> ModelProviderId {
        self.provider_id
    }

    pub fn host(&self) -> &str {
        &self.host
    }

    pub fn path(&self) -> &str {
        &self.path
    }

    pub fn port(&self) -> u16 {
        self.port
    }

    pub fn use_tls(&self) -> bool {
        self.use_tls
    }

    pub fn method(&self) -> &'static str {
        self.method.as_str()
    }

    pub fn body(&self) -> &[u8] {
        &self.body
    }

    pub fn timeout_ms(&self) -> u32 {
        self.timeout_ms
    }

    pub fn max_response_bytes(&self) -> usize {
        self.max_response_bytes
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum ModelHttpMethod {
    Get,
    Post,
}

impl ModelHttpMethod {
    fn as_str(self) -> &'static str {
        match self {
            Self::Get => "GET",
            Self::Post => "POST",
        }
    }
}

#[derive(Debug)]
pub struct ModelCancellation {
    state: AtomicU8,
    #[cfg(windows)]
    native_request_handle: AtomicUsize,
}

impl ModelCancellation {
    fn new() -> Self {
        Self {
            state: AtomicU8::new(CONTROL_ACTIVE),
            #[cfg(windows)]
            native_request_handle: AtomicUsize::new(0),
        }
    }

    pub fn is_cancelled(&self) -> bool {
        matches!(
            self.state.load(Ordering::Acquire),
            CONTROL_CANCELLED | CONTROL_TIMED_OUT
        )
    }

    pub fn cancellation_error(&self, provider_id: ModelProviderId) -> Option<ModelGatewayError> {
        match self.state.load(Ordering::Acquire) {
            CONTROL_CANCELLED => Some(
                ModelGatewayError::new(
                    "PROVIDER_CANCELLED",
                    format!("{} request was cancelled", provider_id.label()),
                )
                .for_provider(provider_id),
            ),
            CONTROL_TIMED_OUT => {
                let mut error = ModelGatewayError::new(
                    "PROVIDER_TIMEOUT",
                    format!("{} request timed out", provider_id.label()),
                )
                .for_provider(provider_id);
                error.retriable = true;
                Some(error)
            }
            _ => None,
        }
    }

    fn cancel(&self) -> bool {
        let changed = self
            .state
            .compare_exchange(
                CONTROL_ACTIVE,
                CONTROL_CANCELLED,
                Ordering::AcqRel,
                Ordering::Acquire,
            )
            .is_ok();
        if changed {
            self.close_native_request();
        }
        changed
    }

    fn time_out(&self) {
        if self
            .state
            .compare_exchange(
                CONTROL_ACTIVE,
                CONTROL_TIMED_OUT,
                Ordering::AcqRel,
                Ordering::Acquire,
            )
            .is_ok()
        {
            self.close_native_request();
        }
    }

    fn complete(&self) {
        let _ = self.state.compare_exchange(
            CONTROL_ACTIVE,
            CONTROL_COMPLETE,
            Ordering::AcqRel,
            Ordering::Acquire,
        );
        self.close_native_request();
    }

    #[cfg(windows)]
    pub(crate) fn install_native_request(
        &self,
        handle: *mut core::ffi::c_void,
        provider_id: ModelProviderId,
    ) -> Result<(), ModelGatewayError> {
        use windows_sys::Win32::Networking::WinHttp::WinHttpCloseHandle;

        let raw = handle as usize;
        if self
            .native_request_handle
            .compare_exchange(0, raw, Ordering::AcqRel, Ordering::Acquire)
            .is_err()
        {
            unsafe { WinHttpCloseHandle(handle) };
            return Err(ModelGatewayError::new(
                "PROVIDER_PROTOCOL",
                "native request handle was registered twice",
            )
            .for_provider(provider_id));
        }
        if let Some(error) = self.cancellation_error(provider_id) {
            self.close_native_request();
            return Err(error);
        }
        Ok(())
    }

    #[cfg(windows)]
    pub(crate) fn close_native_request(&self) {
        use windows_sys::Win32::Networking::WinHttp::WinHttpCloseHandle;

        let raw = self.native_request_handle.swap(0, Ordering::AcqRel);
        if raw != 0 {
            unsafe { WinHttpCloseHandle(raw as *mut core::ffi::c_void) };
        }
    }

    #[cfg(not(windows))]
    fn close_native_request(&self) {}
}

#[derive(Clone)]
pub struct ModelExecutionHost {
    secrets: Arc<dyn SecretStore>,
    transport: Arc<dyn ModelTransport>,
    active: Arc<Mutex<HashMap<String, Arc<ModelCancellation>>>>,
    pending: Arc<Mutex<HashMap<String, PendingModelAuthorization>>>,
}

struct PendingModelAuthorization {
    input: ModelExecutionRequest,
    scope: ModelAuthorizationScope,
    expires_at: OffsetDateTime,
}

impl ModelExecutionHost {
    pub fn new(secrets: Arc<dyn SecretStore>) -> Self {
        Self::with_transport(
            secrets,
            Arc::new(super::model_transport::NativeModelTransport::new()),
        )
    }

    pub fn with_transport(
        secrets: Arc<dyn SecretStore>,
        transport: Arc<dyn ModelTransport>,
    ) -> Self {
        Self {
            secrets,
            transport,
            active: Arc::new(Mutex::new(HashMap::new())),
            pending: Arc::new(Mutex::new(HashMap::new())),
        }
    }

    pub fn authorize(
        &self,
        input: ModelExecutionRequest,
        scope: ModelAuthorizationScope,
    ) -> Result<ModelRequestAuthorization, ModelGatewayError> {
        let prepared = prepare_request(&input)?;
        let provider_id = prepared.provider_id;
        let now = OffsetDateTime::now_utc();
        let expires_at = now + time::Duration::seconds(MODEL_AUTHORIZATION_TTL_SECONDS);
        let mut pending = self.pending.lock().map_err(|_| {
            ModelGatewayError::new(
                "MODEL_EXECUTION_STATE_UNAVAILABLE",
                "model authorization state is unavailable",
            )
        })?;
        pending.retain(|_, authorization| authorization.expires_at > now);
        if pending.len() >= MAX_PENDING_MODEL_AUTHORIZATIONS {
            return Err(ModelGatewayError::new(
                "MODEL_AUTHORIZATION_CAPACITY",
                "too many model requests are waiting for execution",
            )
            .for_provider(provider_id));
        }
        if pending
            .values()
            .any(|authorization| authorization.input.request_id == input.request_id)
            || self
                .active
                .lock()
                .map_err(|_| {
                    ModelGatewayError::new(
                        "MODEL_EXECUTION_STATE_UNAVAILABLE",
                        "model execution state is unavailable",
                    )
                })?
                .contains_key(&input.request_id)
        {
            return Err(ModelGatewayError::new(
                "MODEL_REQUEST_ID_IN_USE",
                "model request ID is already authorized or active",
            )
            .for_provider(provider_id));
        }
        let authorization_id = format!("model-auth-{}", Uuid::new_v4().simple());
        pending.insert(
            authorization_id.clone(),
            PendingModelAuthorization {
                input: input.clone(),
                scope,
                expires_at,
            },
        );
        Ok(ModelRequestAuthorization {
            schema_version: REQUEST_SCHEMA_VERSION,
            authorization_id,
            request_id: input.request_id,
            provider_id,
            expires_at: expires_at
                .format(&Rfc3339)
                .expect("RFC 3339 formatting supports OffsetDateTime"),
        })
    }

    pub fn take_authorized_request(
        &self,
        authorization_id: &str,
        project_id: &str,
        head_commit_id: &str,
    ) -> Result<ModelExecutionRequest, ModelGatewayError> {
        validate_authorization_id(authorization_id)?;
        let authorization = self
            .pending
            .lock()
            .map_err(|_| {
                ModelGatewayError::new(
                    "MODEL_EXECUTION_STATE_UNAVAILABLE",
                    "model authorization state is unavailable",
                )
            })?
            .remove(authorization_id)
            .ok_or_else(|| {
                ModelGatewayError::new(
                    "MODEL_AUTHORIZATION_INVALID",
                    "model request authorization is unknown or already consumed",
                )
            })?;
        let provider_id = authorization.input.configuration.provider_id;
        if authorization.expires_at <= OffsetDateTime::now_utc() {
            return Err(ModelGatewayError::new(
                "MODEL_AUTHORIZATION_EXPIRED",
                "model request authorization has expired",
            )
            .for_provider(provider_id));
        }
        if authorization.scope.project_id != project_id
            || authorization.scope.base_commit_id != head_commit_id
        {
            return Err(ModelGatewayError::new(
                "MODEL_AUTHORIZATION_STALE",
                "the project changed after the model request was authorized",
            )
            .for_provider(provider_id));
        }
        Ok(authorization.input)
    }

    pub fn list_ollama_models(&self) -> Result<OllamaModelList, ModelGatewayError> {
        let prepared = PreparedModelRequest {
            request_id: "ollama-model-list".into(),
            provider_id: ModelProviderId::Ollama,
            host: "127.0.0.1".into(),
            path: "/v1/models".into(),
            port: 11_434,
            use_tls: false,
            method: ModelHttpMethod::Get,
            body: Vec::new(),
            timeout_ms: OLLAMA_DISCOVERY_TIMEOUT_MS,
            max_response_bytes: OLLAMA_DISCOVERY_MAX_BYTES,
        };
        let control = ModelCancellation::new();
        let mut sink = CollectingResponseSink::new(prepared.max_response_bytes());
        self.transport
            .execute(&prepared, None, &control, &mut sink)
            .map_err(|error| error.for_provider(ModelProviderId::Ollama))?;
        let (head, body) = sink.finish(ModelProviderId::Ollama)?;
        if !(200..300).contains(&head.status) {
            return Err(provider_http_error(
                ModelProviderId::Ollama,
                &head,
                &body,
                None,
            ));
        }
        parse_ollama_models(&body)
    }

    pub fn execute_stream<F>(
        &self,
        input: ModelExecutionRequest,
        emit: F,
    ) -> Result<ModelExecutionSummary, ModelGatewayError>
    where
        F: FnMut(ModelStreamEvent) -> Result<(), ModelGatewayError>,
    {
        self.execute_stream_after_ready(input, || {}, emit)
    }

    pub fn execute_stream_after_ready<F, R>(
        &self,
        input: ModelExecutionRequest,
        ready: R,
        mut emit: F,
    ) -> Result<ModelExecutionSummary, ModelGatewayError>
    where
        F: FnMut(ModelStreamEvent) -> Result<(), ModelGatewayError>,
        R: FnOnce(),
    {
        let prepared = prepare_request(&input)?;
        let provider_id = prepared.provider_id;
        let secret = if provider_id == ModelProviderId::Ollama {
            None
        } else {
            let reference = SecretReference::parse(input.configuration.credential_ref.clone())?;
            Some(self.secrets.resolve(&reference)?.ok_or_else(|| {
                ModelGatewayError::new(
                    "PROVIDER_CREDENTIAL_MISSING",
                    format!("No credential is stored for {reference}"),
                )
                .for_provider(provider_id)
            })?)
        };
        let control = Arc::new(ModelCancellation::new());
        {
            let mut active = self.active.lock().map_err(|_| {
                ModelGatewayError::new(
                    "MODEL_EXECUTION_STATE_UNAVAILABLE",
                    "model execution state is unavailable",
                )
            })?;
            if active.contains_key(&prepared.request_id) {
                return Err(ModelGatewayError::new(
                    "MODEL_REQUEST_ID_IN_USE",
                    "model request ID is already active",
                )
                .for_provider(provider_id));
            }
            active.insert(prepared.request_id.clone(), control.clone());
        }
        let _guard = ActiveRequestGuard {
            active: self.active.clone(),
            request_id: prepared.request_id.clone(),
            control: control.clone(),
        };
        ready();
        let (watchdog_done, watchdog_wait) = mpsc::sync_channel(1);
        let timeout_control = control.clone();
        let timeout_ms = prepared.timeout_ms;
        std::thread::spawn(move || {
            if watchdog_wait
                .recv_timeout(Duration::from_millis(u64::from(timeout_ms)))
                .is_err()
            {
                timeout_control.time_out();
            }
        });

        let mut sink = StreamingGatewaySink::new(
            prepared.request_id.clone(),
            provider_id,
            secret.as_ref().map(SecretValue::expose_secret),
            &mut emit,
        );
        let transport_result =
            self.transport
                .execute(&prepared, secret.as_ref(), &control, &mut sink);
        let _ = watchdog_done.send(());
        if let Some(error) = control.cancellation_error(provider_id) {
            return Err(error);
        }
        transport_result.map_err(|error| error.for_provider(provider_id))?;
        sink.finish()
    }

    pub fn cancel(&self, request_id: impl Into<String>) -> CancelModelRequestResponse {
        let request_id = request_id.into();
        let active_cancelled = self
            .active
            .lock()
            .ok()
            .and_then(|active| active.get(&request_id).cloned())
            .is_some_and(|control| control.cancel());
        let pending_cancelled = self
            .pending
            .lock()
            .ok()
            .and_then(|mut pending| {
                let authorization_id = pending.iter().find_map(|(id, authorization)| {
                    (authorization.input.request_id == request_id).then(|| id.clone())
                })?;
                pending.remove(&authorization_id)
            })
            .is_some();
        CancelModelRequestResponse {
            schema_version: REQUEST_SCHEMA_VERSION,
            request_id,
            cancelled: active_cancelled || pending_cancelled,
        }
    }

    pub fn cancel_all(&self) -> usize {
        let controls = self
            .active
            .lock()
            .map(|active| active.values().cloned().collect::<Vec<_>>())
            .unwrap_or_default();
        let active = controls
            .into_iter()
            .filter(|control| control.cancel())
            .count();
        let pending = self
            .pending
            .lock()
            .map(|mut pending| {
                let count = pending.len();
                pending.clear();
                count
            })
            .unwrap_or_default();
        active + pending
    }
}

struct ActiveRequestGuard {
    active: Arc<Mutex<HashMap<String, Arc<ModelCancellation>>>>,
    request_id: String,
    control: Arc<ModelCancellation>,
}

impl Drop for ActiveRequestGuard {
    fn drop(&mut self) {
        self.control.complete();
        if let Ok(mut active) = self.active.lock() {
            active.remove(&self.request_id);
        }
    }
}

fn prepare_request(
    input: &ModelExecutionRequest,
) -> Result<PreparedModelRequest, ModelGatewayError> {
    if input.schema_version != REQUEST_SCHEMA_VERSION {
        return Err(ModelGatewayError::new(
            "UNSUPPORTED_SCHEMA",
            format!(
                "Unsupported model execution schema version {}",
                input.schema_version
            ),
        ));
    }
    validate_request_id(&input.request_id)?;
    validate_configuration(&input.configuration)?;
    validate_model_request(input.configuration.provider_id, &input.request)?;
    let endpoint = provider_endpoint(&input.configuration)?;
    let model = input
        .request
        .model
        .as_deref()
        .map(str::trim)
        .filter(|value| !value.is_empty())
        .unwrap_or(input.configuration.default_model.trim());
    if model.is_empty() || model.len() > 256 {
        return Err(invalid_request(
            input.configuration.provider_id,
            "model must contain 1-256 characters",
        ));
    }
    let body = build_request_body(input.configuration.provider_id, model, &input.request)?;
    let body = serde_json::to_vec(&body).map_err(|_| {
        invalid_request(
            input.configuration.provider_id,
            "model request could not be serialized",
        )
    })?;
    if body.len() > input.configuration.max_request_bytes as usize {
        return Err(invalid_request(
            input.configuration.provider_id,
            format!(
                "request body exceeds the configured {}-byte limit",
                input.configuration.max_request_bytes
            ),
        ));
    }
    Ok(PreparedModelRequest {
        request_id: input.request_id.clone(),
        provider_id: input.configuration.provider_id,
        host: endpoint.host,
        path: endpoint.path,
        port: endpoint.port,
        use_tls: endpoint.use_tls,
        method: ModelHttpMethod::Post,
        body,
        timeout_ms: input.configuration.default_timeout_ms,
        max_response_bytes: MAX_RESPONSE_BYTES,
    })
}

fn validate_request_id(request_id: &str) -> Result<(), ModelGatewayError> {
    if request_id.is_empty()
        || request_id.len() > 128
        || !request_id
            .bytes()
            .all(|byte| byte.is_ascii_alphanumeric() || matches!(byte, b'_' | b'-'))
    {
        return Err(ModelGatewayError::new(
            "INVALID_MODEL_REQUEST",
            "requestId must contain 1-128 ASCII letters, digits, '_' or '-'",
        ));
    }
    Ok(())
}

fn validate_authorization_id(authorization_id: &str) -> Result<(), ModelGatewayError> {
    if authorization_id.is_empty()
        || authorization_id.len() > 128
        || !authorization_id
            .bytes()
            .all(|byte| byte.is_ascii_alphanumeric() || matches!(byte, b'_' | b'-'))
    {
        return Err(ModelGatewayError::new(
            "MODEL_AUTHORIZATION_INVALID",
            "model authorization ID is invalid",
        ));
    }
    Ok(())
}

fn validate_configuration(
    configuration: &ModelProviderConfiguration,
) -> Result<(), ModelGatewayError> {
    let provider_id = configuration.provider_id;
    if configuration.schema_version != REQUEST_SCHEMA_VERSION {
        return Err(ModelGatewayError::new(
            "UNSUPPORTED_SCHEMA",
            format!(
                "Unsupported provider configuration schema version {}",
                configuration.schema_version
            ),
        )
        .for_provider(provider_id));
    }
    if !configuration.enabled {
        return Err(ModelGatewayError::new(
            "PROVIDER_DISABLED",
            format!("{} provider is disabled", provider_id.label()),
        )
        .for_provider(provider_id));
    }
    for (name, value) in [
        ("configuration.id", configuration.id.as_str()),
        (
            "configuration.defaultModel",
            configuration.default_model.as_str(),
        ),
        ("configuration.updatedAt", configuration.updated_at.as_str()),
    ] {
        if value.trim().is_empty() {
            return Err(configuration_error(
                provider_id,
                format!("{name} must not be empty"),
            ));
        }
    }
    if OffsetDateTime::parse(&configuration.updated_at, &Rfc3339).is_err() {
        return Err(configuration_error(
            provider_id,
            "configuration.updatedAt must be an RFC 3339 timestamp",
        ));
    }
    let credential_ref = SecretReference::parse(configuration.credential_ref.clone())?;
    let expected_credential_ref = format!("secret://providers/{provider_id}/default");
    if credential_ref.as_str() != expected_credential_ref {
        return Err(configuration_error(
            provider_id,
            "credentialRef must match the provider's fixed default credential slot",
        ));
    }
    if !(1..=MAX_TIMEOUT_MS).contains(&configuration.default_timeout_ms) {
        return Err(configuration_error(
            provider_id,
            format!("defaultTimeoutMs must be between 1 and {MAX_TIMEOUT_MS}"),
        ));
    }
    if !(1..=MAX_REQUEST_BYTES).contains(&configuration.max_request_bytes) {
        return Err(configuration_error(
            provider_id,
            format!("maxRequestBytes must be between 1 and {MAX_REQUEST_BYTES}"),
        ));
    }
    if provider_id != ModelProviderId::Qwen && configuration.qwen.is_some() {
        return Err(configuration_error(
            provider_id,
            "qwen configuration is only valid for the Qwen provider",
        ));
    }
    if let Some(qwen) = &configuration.qwen {
        if let Some(workspace_id) = &qwen.workspace_id
            && (workspace_id.is_empty()
                || workspace_id.len() > 128
                || !workspace_id
                    .bytes()
                    .all(|byte| byte.is_ascii_alphanumeric() || matches!(byte, b'_' | b'-')))
        {
            return Err(configuration_error(
                provider_id,
                "Qwen workspaceId contains unsupported characters",
            ));
        }
        if matches!(
            qwen.region,
            QwenDeploymentRegion::Germany | QwenDeploymentRegion::Japan
        ) && qwen.workspace_id.is_none()
        {
            return Err(configuration_error(
                provider_id,
                "Qwen Germany and Japan regions require a workspaceId",
            ));
        }
    }
    Ok(())
}

fn validate_model_request(
    provider_id: ModelProviderId,
    request: &ModelRequest,
) -> Result<(), ModelGatewayError> {
    if request.messages.is_empty() {
        return Err(invalid_request(
            provider_id,
            "at least one message is required",
        ));
    }
    for (index, message) in request.messages.iter().enumerate() {
        if message
            .name
            .as_ref()
            .is_some_and(|name| name.trim().is_empty())
        {
            return Err(invalid_request(
                provider_id,
                format!("messages[{index}].name must not be empty"),
            ));
        }
        if message.role == ModelMessageRole::Tool
            && message
                .tool_call_id
                .as_ref()
                .is_none_or(|id| id.trim().is_empty())
        {
            return Err(invalid_request(
                provider_id,
                format!("messages[{index}] tool message requires toolCallId"),
            ));
        }
        if message.role != ModelMessageRole::Assistant
            && (message.reasoning_content.is_some() || message.tool_calls.is_some())
        {
            return Err(invalid_request(
                provider_id,
                format!("messages[{index}] reasoningContent/toolCalls require assistant role"),
            ));
        }
    }
    if request.max_output_tokens.is_some_and(|value| value == 0) {
        return Err(invalid_request(
            provider_id,
            "maxOutputTokens must be positive",
        ));
    }
    if request
        .temperature
        .is_some_and(|value| !value.is_finite() || !(0.0..=2.0).contains(&value))
    {
        return Err(invalid_request(
            provider_id,
            "temperature must be between 0 and 2",
        ));
    }
    if request
        .top_p
        .is_some_and(|value| !value.is_finite() || !(0.0..=1.0).contains(&value))
    {
        return Err(invalid_request(provider_id, "topP must be between 0 and 1"));
    }
    if let Some(stop) = &request.stop {
        let values = stop.values();
        let maximum = if provider_id == ModelProviderId::Kimi {
            5
        } else {
            16
        };
        if values.is_empty()
            || values.len() > maximum
            || values.iter().any(|value| value.is_empty())
        {
            return Err(invalid_request(
                provider_id,
                format!("stop must contain 1-{maximum} non-empty strings"),
            ));
        }
    }
    if provider_id == ModelProviderId::Minimax
        && request.response_format == Some(ModelResponseFormat::JsonObject)
    {
        return Err(invalid_request(
            provider_id,
            "MiniMax has no verified json_object capability",
        ));
    }
    if let Some(tools) = &request.tools {
        if tools.is_empty() || tools.len() > 128 {
            return Err(invalid_request(
                provider_id,
                "tools must contain 1-128 definitions",
            ));
        }
        for tool in tools {
            if tool.kind != "function" || !valid_tool_name(&tool.function.name) {
                return Err(invalid_request(
                    provider_id,
                    "tool definitions require type=function and a safe 1-64 character name",
                ));
            }
        }
    } else if request
        .tool_choice
        .as_ref()
        .is_some_and(|choice| *choice != ModelToolChoice::None)
    {
        return Err(invalid_request(provider_id, "toolChoice requires tools"));
    }
    Ok(())
}

fn valid_tool_name(value: &str) -> bool {
    !value.is_empty()
        && value.len() <= 64
        && value
            .bytes()
            .all(|byte| byte.is_ascii_alphanumeric() || matches!(byte, b'_' | b'-'))
}

struct ProviderEndpoint {
    host: String,
    path: String,
    port: u16,
    use_tls: bool,
}

fn provider_endpoint(
    configuration: &ModelProviderConfiguration,
) -> Result<ProviderEndpoint, ModelGatewayError> {
    let endpoint = match configuration.provider_id {
        ModelProviderId::Deepseek => ("api.deepseek.com".to_string(), "/chat/completions"),
        ModelProviderId::Kimi => ("api.moonshot.cn".to_string(), "/v1/chat/completions"),
        ModelProviderId::Minimax => ("api.minimaxi.com".to_string(), "/v1/chat/completions"),
        ModelProviderId::Ollama => ("127.0.0.1".to_string(), "/v1/chat/completions"),
        ModelProviderId::Qwen => {
            let default = QwenProviderConfiguration {
                region: QwenDeploymentRegion::China,
                workspace_id: None,
            };
            let qwen = configuration.qwen.as_ref().unwrap_or(&default);
            let host = match (&qwen.region, qwen.workspace_id.as_deref()) {
                (QwenDeploymentRegion::Us, _) => "dashscope-us.aliyuncs.com".to_string(),
                (QwenDeploymentRegion::China, None) => "dashscope.aliyuncs.com".to_string(),
                (QwenDeploymentRegion::Singapore, None) => {
                    "dashscope-intl.aliyuncs.com".to_string()
                }
                (region, Some(workspace_id)) => {
                    let suffix = match region {
                        QwenDeploymentRegion::China => "cn-beijing.maas.aliyuncs.com",
                        QwenDeploymentRegion::Singapore => "ap-southeast-1.maas.aliyuncs.com",
                        QwenDeploymentRegion::Germany => "eu-central-1.maas.aliyuncs.com",
                        QwenDeploymentRegion::Japan => "ap-northeast-1.maas.aliyuncs.com",
                        QwenDeploymentRegion::Us => unreachable!("US handled above"),
                    };
                    format!("{workspace_id}.{suffix}")
                }
                _ => {
                    return Err(configuration_error(
                        ModelProviderId::Qwen,
                        "Qwen region requires a workspaceId",
                    ));
                }
            };
            (host, "/compatible-mode/v1/chat/completions")
        }
    };
    Ok(ProviderEndpoint {
        host: endpoint.0,
        path: endpoint.1.to_string(),
        port: if configuration.provider_id == ModelProviderId::Ollama {
            11_434
        } else {
            443
        },
        use_tls: configuration.provider_id != ModelProviderId::Ollama,
    })
}

fn build_request_body(
    provider_id: ModelProviderId,
    model: &str,
    request: &ModelRequest,
) -> Result<Value, ModelGatewayError> {
    if provider_id == ModelProviderId::Kimi
        && model.to_ascii_lowercase().contains("k2.7-code")
        && request
            .reasoning
            .as_ref()
            .is_some_and(|reasoning| reasoning.mode == ReasoningMode::Disabled)
    {
        return Err(invalid_request(
            provider_id,
            "kimi-k2.7-code always has thinking enabled",
        ));
    }
    let messages = request
        .messages
        .iter()
        .map(serialize_message)
        .collect::<Vec<_>>();
    let mut body = Map::from_iter([
        ("model".into(), Value::String(model.to_string())),
        ("messages".into(), Value::Array(messages)),
        ("stream".into(), Value::Bool(true)),
        ("stream_options".into(), json!({ "include_usage": true })),
    ]);
    if let Some(maximum) = request.max_output_tokens {
        let field = match provider_id {
            ModelProviderId::Deepseek | ModelProviderId::Kimi | ModelProviderId::Ollama => {
                "max_tokens"
            }
            ModelProviderId::Qwen | ModelProviderId::Minimax => "max_completion_tokens",
        };
        body.insert(field.into(), Value::from(maximum));
    }
    if let Some(temperature) = request.temperature {
        body.insert("temperature".into(), Value::from(temperature));
    }
    if let Some(top_p) = request.top_p {
        body.insert("top_p".into(), Value::from(top_p));
    }
    if let Some(stop) = &request.stop {
        body.insert(
            "stop".into(),
            serde_json::to_value(stop).expect("stop is serializable"),
        );
    }
    if let Some(format) = request.response_format {
        body.insert("response_format".into(), json!({ "type": format.as_str() }));
    }
    if let Some(tools) = &request.tools {
        body.insert(
            "tools".into(),
            serde_json::to_value(tools).expect("tools are serializable"),
        );
    }
    if let Some(choice) = &request.tool_choice {
        body.insert("tool_choice".into(), Value::String(choice.as_str().into()));
    }
    apply_reasoning_dialect(provider_id, request.reasoning.as_ref(), &mut body);
    Ok(Value::Object(body))
}

fn serialize_message(message: &ModelMessage) -> Value {
    let mut output = Map::from_iter([
        ("role".into(), Value::String(message.role.as_str().into())),
        ("content".into(), Value::String(message.content.clone())),
    ]);
    if let Some(name) = &message.name {
        output.insert("name".into(), Value::String(name.clone()));
    }
    if let Some(reasoning) = &message.reasoning_content {
        output.insert("reasoning_content".into(), Value::String(reasoning.clone()));
    }
    if let Some(call_id) = &message.tool_call_id {
        output.insert("tool_call_id".into(), Value::String(call_id.clone()));
    }
    if let Some(calls) = &message.tool_calls {
        output.insert(
            "tool_calls".into(),
            serde_json::to_value(calls).expect("tool calls are serializable"),
        );
    }
    Value::Object(output)
}

fn apply_reasoning_dialect(
    provider_id: ModelProviderId,
    reasoning: Option<&ReasoningOptions>,
    body: &mut Map<String, Value>,
) {
    if provider_id == ModelProviderId::Minimax {
        body.insert("reasoning_split".into(), Value::Bool(true));
        if let Some(reasoning) = reasoning {
            let kind = if reasoning.mode == ReasoningMode::Disabled {
                "disabled"
            } else {
                "adaptive"
            };
            body.insert("thinking".into(), json!({ "type": kind }));
        }
        return;
    }
    let Some(reasoning) = reasoning else {
        return;
    };
    match provider_id {
        ModelProviderId::Qwen => {
            if reasoning.mode != ReasoningMode::Adaptive {
                body.insert(
                    "enable_thinking".into(),
                    Value::Bool(reasoning.mode == ReasoningMode::Enabled),
                );
            }
            if let Some(preserve) = reasoning.preserve {
                body.insert("preserve_thinking".into(), Value::Bool(preserve));
            }
            if let Some(effort) = reasoning.effort {
                body.insert(
                    "reasoning_effort".into(),
                    Value::String(effort.as_str().into()),
                );
            }
        }
        ModelProviderId::Kimi => {
            let mut thinking = Map::from_iter([(
                "type".into(),
                Value::String(
                    if reasoning.mode == ReasoningMode::Disabled {
                        "disabled"
                    } else {
                        "enabled"
                    }
                    .into(),
                ),
            )]);
            if reasoning.preserve == Some(true) {
                thinking.insert("keep".into(), Value::String("all".into()));
            }
            body.insert("thinking".into(), Value::Object(thinking));
        }
        ModelProviderId::Deepseek => {
            if reasoning.mode != ReasoningMode::Adaptive {
                body.insert(
                    "thinking".into(),
                    json!({ "type": reasoning.mode.as_str() }),
                );
            }
            if let Some(effort) = reasoning.effort {
                body.insert(
                    "reasoning_effort".into(),
                    Value::String(effort.as_str().into()),
                );
            }
        }
        ModelProviderId::Ollama => {
            let effort = match (reasoning.mode, reasoning.effort) {
                (ReasoningMode::Disabled, _) => Some("none"),
                (_, Some(effort)) => Some(effort.as_str()),
                (ReasoningMode::Enabled, None) => Some("medium"),
                (ReasoningMode::Adaptive, None) => None,
            };
            if let Some(effort) = effort {
                body.insert("reasoning_effort".into(), Value::String(effort.into()));
            }
        }
        ModelProviderId::Minimax => unreachable!("handled above"),
    }
}

fn invalid_request(provider_id: ModelProviderId, message: impl Into<String>) -> ModelGatewayError {
    ModelGatewayError::new("INVALID_MODEL_REQUEST", message).for_provider(provider_id)
}

fn configuration_error(
    provider_id: ModelProviderId,
    message: impl Into<String>,
) -> ModelGatewayError {
    ModelGatewayError::new("PROVIDER_CONFIGURATION", message).for_provider(provider_id)
}

struct CollectingResponseSink {
    head: Option<ModelHttpResponseHead>,
    body: Vec<u8>,
    maximum: usize,
}

impl CollectingResponseSink {
    fn new(maximum: usize) -> Self {
        Self {
            head: None,
            body: Vec::new(),
            maximum,
        }
    }

    fn finish(
        self,
        provider_id: ModelProviderId,
    ) -> Result<(ModelHttpResponseHead, Vec<u8>), ModelGatewayError> {
        let head = self.head.ok_or_else(|| {
            ModelGatewayError::new(
                "PROVIDER_PROTOCOL",
                "provider transport ended before response headers",
            )
            .for_provider(provider_id)
        })?;
        Ok((head, self.body))
    }
}

impl ModelResponseSink for CollectingResponseSink {
    fn begin(&mut self, head: ModelHttpResponseHead) -> Result<(), ModelGatewayError> {
        if self.head.replace(head).is_some() {
            return Err(ModelGatewayError::new(
                "PROVIDER_PROTOCOL",
                "provider returned response headers twice",
            ));
        }
        Ok(())
    }

    fn chunk(&mut self, chunk: &[u8]) -> Result<(), ModelGatewayError> {
        if self.body.len().saturating_add(chunk.len()) > self.maximum {
            return Err(ModelGatewayError::new(
                "PROVIDER_PROTOCOL",
                "Ollama model list exceeded the configured size limit",
            ));
        }
        self.body.extend_from_slice(chunk);
        Ok(())
    }
}

fn parse_ollama_models(body: &[u8]) -> Result<OllamaModelList, ModelGatewayError> {
    let root: Value = serde_json::from_slice(body)
        .map_err(|_| ollama_protocol_error("Ollama model list is not valid JSON"))?;
    let models = root
        .as_object()
        .and_then(|value| value.get("data"))
        .and_then(Value::as_array)
        .ok_or_else(|| ollama_protocol_error("Ollama model list must contain a data array"))?;
    if models.len() > 10_000 {
        return Err(ollama_protocol_error(
            "Ollama model list contains too many entries",
        ));
    }
    let mut normalized = BTreeMap::new();
    for (index, value) in models.iter().enumerate() {
        let model = value
            .as_object()
            .ok_or_else(|| ollama_protocol_error(format!("data[{index}] must be an object")))?;
        let id = model
            .get("id")
            .and_then(Value::as_str)
            .filter(|id| !id.is_empty() && id.len() <= 256 && !id.chars().any(char::is_control))
            .ok_or_else(|| {
                ollama_protocol_error(format!("data[{index}].id must be a safe model ID"))
            })?
            .to_string();
        let created = match model.get("created") {
            Some(value) => Some(value.as_u64().ok_or_else(|| {
                ollama_protocol_error(format!("data[{index}].created must be an integer"))
            })?),
            None => None,
        };
        let owned_by = match model.get("owned_by") {
            Some(value) => {
                let value = value
                    .as_str()
                    .filter(|value| value.len() <= 128)
                    .ok_or_else(|| {
                        ollama_protocol_error(format!(
                            "data[{index}].owned_by must be a short string"
                        ))
                    })?;
                Some(value.to_string())
            }
            None => None,
        };
        if normalized
            .insert(
                id.clone(),
                OllamaModelInfo {
                    id,
                    created,
                    owned_by,
                },
            )
            .is_some()
        {
            return Err(ollama_protocol_error(
                "Ollama model list contains duplicate IDs",
            ));
        }
    }
    Ok(OllamaModelList {
        schema_version: REQUEST_SCHEMA_VERSION,
        endpoint: "127.0.0.1:11434/v1".into(),
        models: normalized.into_values().collect(),
    })
}

fn ollama_protocol_error(message: impl Into<String>) -> ModelGatewayError {
    ModelGatewayError::new("PROVIDER_PROTOCOL", message).for_provider(ModelProviderId::Ollama)
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum StreamContentMode {
    Delta,
    Cumulative,
}

struct StreamingGatewaySink<'a> {
    request_id: String,
    provider_id: ModelProviderId,
    secret: Option<&'a str>,
    emit: &'a mut dyn FnMut(ModelStreamEvent) -> Result<(), ModelGatewayError>,
    head: Option<ModelHttpResponseHead>,
    error_body: Vec<u8>,
    decoder: SseDecoder,
    started: bool,
    response_id: Option<String>,
    model: Option<String>,
    cumulative_content: String,
    cumulative_reasoning: String,
    finish_reason: ModelFinishReason,
    usage: Option<ModelUsage>,
}

impl<'a> StreamingGatewaySink<'a> {
    fn new(
        request_id: String,
        provider_id: ModelProviderId,
        secret: Option<&'a str>,
        emit: &'a mut dyn FnMut(ModelStreamEvent) -> Result<(), ModelGatewayError>,
    ) -> Self {
        Self {
            request_id,
            provider_id,
            secret,
            emit,
            head: None,
            error_body: Vec::new(),
            decoder: SseDecoder::new(),
            started: false,
            response_id: None,
            model: None,
            cumulative_content: String::new(),
            cumulative_reasoning: String::new(),
            finish_reason: ModelFinishReason::Unknown,
            usage: None,
        }
    }

    fn finish(mut self) -> Result<ModelExecutionSummary, ModelGatewayError> {
        let Some(head) = self.head.clone() else {
            return Err(ModelGatewayError::new(
                "PROVIDER_PROTOCOL",
                "provider transport ended before response headers",
            )
            .for_provider(self.provider_id));
        };
        if !(200..300).contains(&head.status) {
            return Err(provider_http_error(
                self.provider_id,
                &head,
                &self.error_body,
                self.secret,
            ));
        }
        for event in self.decoder.finish()? {
            self.process_sse_event(&event)?;
        }
        if !self.started {
            return Err(ModelGatewayError::new(
                "PROVIDER_PROTOCOL",
                "stream ended before an output chunk was received",
            )
            .for_provider(self.provider_id));
        }
        (self.emit)(ModelStreamEvent::Finish {
            reason: self.finish_reason,
        })?;
        Ok(ModelExecutionSummary {
            schema_version: REQUEST_SCHEMA_VERSION,
            request_id: self.request_id,
            response_id: self.response_id.expect("started stream has response ID"),
            provider_id: self.provider_id,
            model: self.model.expect("started stream has model"),
            content: self.cumulative_content,
            reasoning_content: self.cumulative_reasoning,
            finish_reason: self.finish_reason,
            usage: self.usage,
        })
    }

    fn process_sse_event(&mut self, event: &[u8]) -> Result<(), ModelGatewayError> {
        let value: Value = serde_json::from_slice(event).map_err(|_| {
            ModelGatewayError::new("PROVIDER_PROTOCOL", "SSE data is not valid JSON")
                .for_provider(self.provider_id)
        })?;
        let root = value.as_object().ok_or_else(|| {
            ModelGatewayError::new("PROVIDER_PROTOCOL", "stream chunk must be an object")
                .for_provider(self.provider_id)
        })?;
        let chunk_id =
            optional_non_empty_string(root.get("id"), "stream chunk id", self.provider_id)?;
        let chunk_model =
            optional_non_empty_string(root.get("model"), "stream chunk model", self.provider_id)?;
        if self.response_id.is_none() {
            self.response_id = chunk_id;
        }
        if self.model.is_none() {
            self.model = chunk_model;
        }
        let choices = match root.get("choices") {
            None => &[][..],
            Some(Value::Array(values)) => values.as_slice(),
            Some(_) => {
                return Err(ModelGatewayError::new(
                    "PROVIDER_PROTOCOL",
                    "stream choices must be an array",
                )
                .for_provider(self.provider_id));
            }
        };
        let mut content = None;
        let mut reasoning = None;
        let mut reasoning_details = None;
        let mut tool_calls = Vec::new();
        let mut finish = None;
        if let Some(choice) = choices.first() {
            let choice = choice.as_object().ok_or_else(|| {
                ModelGatewayError::new("PROVIDER_PROTOCOL", "stream choice must be an object")
                    .for_provider(self.provider_id)
            })?;
            let delta = choice
                .get("delta")
                .and_then(Value::as_object)
                .ok_or_else(|| {
                    ModelGatewayError::new(
                        "PROVIDER_PROTOCOL",
                        "stream choice delta must be an object",
                    )
                    .for_provider(self.provider_id)
                })?;
            content =
                optional_nullable_string(delta.get("content"), "delta.content", self.provider_id)?;
            reasoning = optional_nullable_string(
                delta.get("reasoning_content"),
                "delta.reasoning_content",
                self.provider_id,
            )?
            .or(optional_nullable_string(
                delta.get("reasoning"),
                "delta.reasoning",
                self.provider_id,
            )?);
            reasoning_details =
                parse_reasoning_details(delta.get("reasoning_details"), self.provider_id)?;
            tool_calls = parse_tool_call_deltas(delta.get("tool_calls"), self.provider_id)?;
            if choice
                .get("finish_reason")
                .is_some_and(|value| !value.is_null())
            {
                finish = Some(ModelFinishReason::from_value(choice.get("finish_reason")));
            }
        }
        let has_output = content.as_deref().is_some_and(|value| !value.is_empty())
            || reasoning.as_deref().is_some_and(|value| !value.is_empty())
            || reasoning_details
                .as_deref()
                .is_some_and(|value| !value.is_empty())
            || !tool_calls.is_empty()
            || finish.is_some();
        if !self.started && has_output {
            let response_id = self.response_id.clone().ok_or_else(|| {
                ModelGatewayError::new(
                    "PROVIDER_PROTOCOL",
                    "first output chunk must include id and model",
                )
                .for_provider(self.provider_id)
            })?;
            let model = self.model.clone().ok_or_else(|| {
                ModelGatewayError::new(
                    "PROVIDER_PROTOCOL",
                    "first output chunk must include id and model",
                )
                .for_provider(self.provider_id)
            })?;
            self.started = true;
            (self.emit)(ModelStreamEvent::Start {
                request_id: self.request_id.clone(),
                id: response_id,
                provider_id: self.provider_id,
                model,
            })?;
        }
        if let Some(reasoning_details) = reasoning_details {
            let delta = normalize_piece(
                StreamContentMode::Cumulative,
                &mut self.cumulative_reasoning,
                &reasoning_details,
            );
            if !delta.is_empty() {
                (self.emit)(ModelStreamEvent::ReasoningDelta { text: delta })?;
            }
        } else if let Some(reasoning) = reasoning {
            let delta = normalize_piece(
                self.provider_id.stream_mode(),
                &mut self.cumulative_reasoning,
                &reasoning,
            );
            if !delta.is_empty() {
                (self.emit)(ModelStreamEvent::ReasoningDelta { text: delta })?;
            }
        }
        if let Some(content) = content {
            let delta = normalize_piece(
                self.provider_id.stream_mode(),
                &mut self.cumulative_content,
                &content,
            );
            if !delta.is_empty() {
                (self.emit)(ModelStreamEvent::TextDelta { text: delta })?;
            }
        }
        for call in tool_calls {
            (self.emit)(call)?;
        }
        if let Some(usage) = parse_usage(root.get("usage"), self.provider_id)? {
            self.usage = Some(usage.clone());
            (self.emit)(ModelStreamEvent::Usage { usage })?;
        }
        if let Some(finish) = finish {
            self.finish_reason = finish;
        }
        if self.cumulative_content.len() + self.cumulative_reasoning.len() > MAX_OUTPUT_BYTES {
            return Err(ModelGatewayError::new(
                "PROVIDER_PROTOCOL",
                "model output exceeded the hard size limit",
            )
            .for_provider(self.provider_id));
        }
        Ok(())
    }
}

impl ModelResponseSink for StreamingGatewaySink<'_> {
    fn begin(&mut self, head: ModelHttpResponseHead) -> Result<(), ModelGatewayError> {
        if self.head.is_some() {
            return Err(ModelGatewayError::new(
                "PROVIDER_PROTOCOL",
                "provider transport emitted response headers twice",
            )
            .for_provider(self.provider_id));
        }
        if (200..300).contains(&head.status)
            && head
                .header("content-type")
                .is_none_or(|value| !value.to_ascii_lowercase().contains("text/event-stream"))
        {
            return Err(ModelGatewayError::new(
                "PROVIDER_PROTOCOL",
                "streaming response must use text/event-stream",
            )
            .for_provider(self.provider_id));
        }
        self.head = Some(head);
        Ok(())
    }

    fn chunk(&mut self, chunk: &[u8]) -> Result<(), ModelGatewayError> {
        let Some(head) = self.head.as_ref() else {
            return Err(ModelGatewayError::new(
                "PROVIDER_PROTOCOL",
                "provider transport emitted body before response headers",
            )
            .for_provider(self.provider_id));
        };
        if !(200..300).contains(&head.status) {
            let remaining = MAX_ERROR_BYTES.saturating_sub(self.error_body.len());
            self.error_body.extend(&chunk[..chunk.len().min(remaining)]);
            return Ok(());
        }
        for event in self.decoder.feed(chunk)? {
            self.process_sse_event(&event)?;
        }
        Ok(())
    }
}

fn normalize_piece(mode: StreamContentMode, previous: &mut String, current: &str) -> String {
    match mode {
        StreamContentMode::Delta => {
            previous.push_str(current);
            current.to_string()
        }
        StreamContentMode::Cumulative if current.starts_with(previous.as_str()) => {
            let delta = current[previous.len()..].to_string();
            *previous = current.to_string();
            delta
        }
        StreamContentMode::Cumulative if previous.ends_with(current) => String::new(),
        StreamContentMode::Cumulative => {
            previous.push_str(current);
            current.to_string()
        }
    }
}

fn optional_non_empty_string(
    value: Option<&Value>,
    name: &str,
    provider_id: ModelProviderId,
) -> Result<Option<String>, ModelGatewayError> {
    match value {
        None | Some(Value::Null) => Ok(None),
        Some(Value::String(value)) if !value.is_empty() => Ok(Some(value.clone())),
        _ => Err(ModelGatewayError::new(
            "PROVIDER_PROTOCOL",
            format!("{name} must be a non-empty string"),
        )
        .for_provider(provider_id)),
    }
}

fn optional_nullable_string(
    value: Option<&Value>,
    name: &str,
    provider_id: ModelProviderId,
) -> Result<Option<String>, ModelGatewayError> {
    match value {
        None | Some(Value::Null) => Ok(None),
        Some(Value::String(value)) => Ok(Some(value.clone())),
        _ => Err(
            ModelGatewayError::new("PROVIDER_PROTOCOL", format!("{name} must be a string"))
                .for_provider(provider_id),
        ),
    }
}

fn parse_reasoning_details(
    value: Option<&Value>,
    provider_id: ModelProviderId,
) -> Result<Option<String>, ModelGatewayError> {
    let Some(value) = value else {
        return Ok(None);
    };
    if value.is_null() {
        return Ok(None);
    }
    let details = value.as_array().ok_or_else(|| {
        ModelGatewayError::new("PROVIDER_PROTOCOL", "reasoning_details must be an array")
            .for_provider(provider_id)
    })?;
    let mut output = String::new();
    for detail in details {
        let text = detail
            .as_object()
            .and_then(|detail| detail.get("text"))
            .and_then(Value::as_str)
            .ok_or_else(|| {
                ModelGatewayError::new(
                    "PROVIDER_PROTOCOL",
                    "reasoning_details text must be a string",
                )
                .for_provider(provider_id)
            })?;
        output.push_str(text);
    }
    Ok((!output.is_empty()).then_some(output))
}

fn parse_tool_call_deltas(
    value: Option<&Value>,
    provider_id: ModelProviderId,
) -> Result<Vec<ModelStreamEvent>, ModelGatewayError> {
    let Some(value) = value else {
        return Ok(Vec::new());
    };
    if value.is_null() {
        return Ok(Vec::new());
    }
    let calls = value.as_array().ok_or_else(|| {
        ModelGatewayError::new("PROVIDER_PROTOCOL", "delta.tool_calls must be an array")
            .for_provider(provider_id)
    })?;
    calls
        .iter()
        .enumerate()
        .map(|(position, value)| {
            let call = value.as_object().ok_or_else(|| {
                ModelGatewayError::new("PROVIDER_PROTOCOL", "tool call delta must be an object")
                    .for_provider(provider_id)
            })?;
            let index = call
                .get("index")
                .and_then(Value::as_u64)
                .unwrap_or(position as u64);
            let function = call.get("function").and_then(Value::as_object);
            Ok(ModelStreamEvent::ToolCallDelta {
                index,
                id: optional_nullable_string(call.get("id"), "tool call id", provider_id)?,
                name: optional_nullable_string(
                    function.and_then(|value| value.get("name")),
                    "tool call name",
                    provider_id,
                )?,
                arguments: optional_nullable_string(
                    function.and_then(|value| value.get("arguments")),
                    "tool call arguments",
                    provider_id,
                )?,
            })
        })
        .collect()
}

fn parse_usage(
    value: Option<&Value>,
    provider_id: ModelProviderId,
) -> Result<Option<ModelUsage>, ModelGatewayError> {
    let Some(value) = value else {
        return Ok(None);
    };
    if value.is_null() {
        return Ok(None);
    }
    let usage = value.as_object().ok_or_else(|| {
        ModelGatewayError::new("PROVIDER_PROTOCOL", "usage must be an object")
            .for_provider(provider_id)
    })?;
    let input_tokens = first_u64(usage, &["prompt_tokens", "input_tokens"]).unwrap_or(0);
    let output_tokens = first_u64(usage, &["completion_tokens", "output_tokens"]).unwrap_or(0);
    let total_tokens = first_u64(usage, &["total_tokens"]).unwrap_or(input_tokens + output_tokens);
    let prompt_details = usage
        .get("prompt_tokens_details")
        .and_then(Value::as_object);
    let completion_details = usage
        .get("completion_tokens_details")
        .and_then(Value::as_object);
    let cached_input_tokens = first_u64(usage, &["cached_tokens", "prompt_cache_hit_tokens"])
        .or_else(|| prompt_details.and_then(|value| first_u64(value, &["cached_tokens"])));
    let reasoning_tokens =
        completion_details.and_then(|value| first_u64(value, &["reasoning_tokens"]));
    Ok(Some(ModelUsage {
        input_tokens,
        output_tokens,
        total_tokens,
        cached_input_tokens,
        reasoning_tokens,
    }))
}

fn first_u64(values: &Map<String, Value>, names: &[&str]) -> Option<u64> {
    names
        .iter()
        .find_map(|name| values.get(*name).and_then(Value::as_u64))
}

fn provider_http_error(
    provider_id: ModelProviderId,
    head: &ModelHttpResponseHead,
    body: &[u8],
    secret: Option<&str>,
) -> ModelGatewayError {
    let payload = serde_json::from_slice::<Value>(body).ok();
    let nested = payload
        .as_ref()
        .and_then(Value::as_object)
        .and_then(|root| root.get("error").and_then(Value::as_object).or(Some(root)));
    let remote_message = nested
        .and_then(|error| error.get("message"))
        .and_then(Value::as_str)
        .map(str::to_string);
    let remote_code = nested.and_then(|error| {
        error
            .get("code")
            .or_else(|| error.get("type"))
            .and_then(Value::as_str)
            .map(str::to_string)
    });
    let remote_code = remote_code
        .as_deref()
        .map(|value| bounded_redacted(value, secret, 200));
    let normalized_code = remote_code
        .as_deref()
        .unwrap_or_default()
        .to_ascii_lowercase();
    let code = if normalized_code.contains("content_filter")
        || normalized_code.contains("sensitive")
        || normalized_code.contains("safety")
    {
        "PROVIDER_CONTENT_FILTER"
    } else {
        match head.status {
            401 => "PROVIDER_AUTHENTICATION",
            402 => "PROVIDER_QUOTA",
            403 => "PROVIDER_PERMISSION",
            429 => "PROVIDER_RATE_LIMIT",
            400 | 404 | 409 | 422 => "INVALID_MODEL_REQUEST",
            _ => "PROVIDER_SERVER",
        }
    };
    let default_message = format!(
        "{} request failed with HTTP {}",
        provider_id.label(),
        head.status
    );
    let message = bounded_redacted(
        remote_message.as_deref().unwrap_or(&default_message),
        secret,
        1_000,
    );
    let retry_after_ms = head.header("retry-after").and_then(parse_retry_after);
    let remote_request_id = ["x-request-id", "request-id", "x-dashscope-request-id"]
        .iter()
        .find_map(|name| head.header(name))
        .map(|value| bounded_redacted(value, secret, 512));
    ModelGatewayError {
        code,
        message,
        provider_id: Some(provider_id),
        status: Some(head.status),
        remote_code,
        remote_request_id,
        retry_after_ms,
        retriable: head.status == 408 || head.status == 429 || head.status >= 500,
    }
}

fn bounded_redacted(value: &str, secret: Option<&str>, max_chars: usize) -> String {
    let redacted = secret.filter(|secret| !secret.is_empty()).map_or_else(
        || value.to_owned(),
        |secret| value.replace(secret, "[REDACTED]"),
    );
    redacted.chars().take(max_chars).collect()
}

fn parse_retry_after(value: &str) -> Option<u64> {
    if let Ok(seconds) = value.trim().parse::<f64>()
        && seconds.is_finite()
        && seconds >= 0.0
    {
        return Some((seconds * 1_000.0).ceil() as u64);
    }
    OffsetDateTime::parse(value.trim(), &Rfc2822)
        .ok()
        .map(|date| {
            let difference = date - OffsetDateTime::now_utc();
            u64::try_from(difference.whole_milliseconds().max(0)).unwrap_or(u64::MAX)
        })
}

struct SseDecoder {
    buffer: Vec<u8>,
    data_lines: Vec<Vec<u8>>,
    data_bytes: usize,
    done: bool,
}

impl SseDecoder {
    fn new() -> Self {
        Self {
            buffer: Vec::new(),
            data_lines: Vec::new(),
            data_bytes: 0,
            done: false,
        }
    }

    fn feed(&mut self, chunk: &[u8]) -> Result<Vec<Vec<u8>>, ModelGatewayError> {
        if self.done {
            return Ok(Vec::new());
        }
        self.buffer.extend_from_slice(chunk);
        if self.buffer.len() > MAX_SSE_EVENT_BYTES {
            return Err(ModelGatewayError::new(
                "PROVIDER_PROTOCOL",
                "SSE event exceeded the configured size limit",
            ));
        }
        let mut events = Vec::new();
        while let Some(newline) = self.buffer.iter().position(|byte| *byte == b'\n') {
            let mut line = self.buffer.drain(..=newline).collect::<Vec<_>>();
            line.pop();
            if line.last() == Some(&b'\r') {
                line.pop();
            }
            self.process_line(&line, &mut events)?;
            if self.done {
                self.buffer.clear();
                break;
            }
        }
        Ok(events)
    }

    fn finish(&mut self) -> Result<Vec<Vec<u8>>, ModelGatewayError> {
        if self.done {
            return Ok(Vec::new());
        }
        let mut events = Vec::new();
        if !self.buffer.is_empty() {
            let mut line = std::mem::take(&mut self.buffer);
            if line.last() == Some(&b'\r') {
                line.pop();
            }
            self.process_line(&line, &mut events)?;
        }
        self.flush_event(&mut events)?;
        Ok(events)
    }

    fn process_line(
        &mut self,
        line: &[u8],
        events: &mut Vec<Vec<u8>>,
    ) -> Result<(), ModelGatewayError> {
        std::str::from_utf8(line).map_err(|_| {
            ModelGatewayError::new("PROVIDER_PROTOCOL", "SSE stream is not valid UTF-8")
        })?;
        if line.is_empty() {
            return self.flush_event(events);
        }
        if let Some(data) = line.strip_prefix(b"data:") {
            let data = data.strip_prefix(b" ").unwrap_or(data);
            self.data_bytes = self.data_bytes.saturating_add(data.len());
            if self.data_bytes > MAX_SSE_EVENT_BYTES {
                return Err(ModelGatewayError::new(
                    "PROVIDER_PROTOCOL",
                    "SSE event exceeded the configured size limit",
                ));
            }
            self.data_lines.push(data.to_vec());
        }
        Ok(())
    }

    fn flush_event(&mut self, events: &mut Vec<Vec<u8>>) -> Result<(), ModelGatewayError> {
        if self.data_lines.is_empty() {
            return Ok(());
        }
        let mut data = Vec::with_capacity(self.data_bytes + self.data_lines.len());
        for (index, line) in self.data_lines.drain(..).enumerate() {
            if index > 0 {
                data.push(b'\n');
            }
            data.extend(line);
        }
        self.data_bytes = 0;
        if data == b"[DONE]" {
            self.done = true;
        } else {
            events.push(data);
        }
        Ok(())
    }
}

pub use super::model_transport::NativeModelTransport;

#[cfg(test)]
mod tests {
    use std::sync::atomic::{AtomicBool, AtomicUsize};

    use crate::MemorySecretStore;

    use super::*;

    struct ScriptedTransport {
        head: ModelHttpResponseHead,
        chunks: Vec<Vec<u8>>,
        calls: AtomicUsize,
        saw_expected_secret: AtomicBool,
        captured: Mutex<Option<PreparedModelRequest>>,
    }

    impl ScriptedTransport {
        fn sse(chunks: Vec<Vec<u8>>) -> Self {
            Self {
                head: ModelHttpResponseHead {
                    status: 200,
                    headers: BTreeMap::from([(
                        "content-type".into(),
                        "text/event-stream; charset=utf-8".into(),
                    )]),
                },
                chunks,
                calls: AtomicUsize::new(0),
                saw_expected_secret: AtomicBool::new(false),
                captured: Mutex::new(None),
            }
        }
    }

    impl ModelTransport for ScriptedTransport {
        fn execute(
            &self,
            request: &PreparedModelRequest,
            secret: Option<&SecretValue>,
            _cancellation: &ModelCancellation,
            sink: &mut dyn ModelResponseSink,
        ) -> Result<(), ModelGatewayError> {
            self.calls.fetch_add(1, Ordering::Relaxed);
            self.saw_expected_secret.store(
                secret.is_some_and(|value| value.expose_secret() == "host-only-key"),
                Ordering::Relaxed,
            );
            *self.captured.lock().unwrap() = Some(request.clone());
            sink.begin(self.head.clone())?;
            for chunk in &self.chunks {
                sink.chunk(chunk)?;
            }
            Ok(())
        }
    }

    struct BlockingTransport {
        started: Mutex<Option<mpsc::Sender<()>>>,
    }

    impl ModelTransport for BlockingTransport {
        fn execute(
            &self,
            request: &PreparedModelRequest,
            _secret: Option<&SecretValue>,
            cancellation: &ModelCancellation,
            _sink: &mut dyn ModelResponseSink,
        ) -> Result<(), ModelGatewayError> {
            if let Some(started) = self.started.lock().unwrap().take() {
                let _ = started.send(());
            }
            while !cancellation.is_cancelled() {
                std::thread::yield_now();
            }
            Err(cancellation
                .cancellation_error(request.provider_id())
                .expect("blocking transport ends through cancellation"))
        }
    }

    fn configuration(provider_id: ModelProviderId) -> ModelProviderConfiguration {
        ModelProviderConfiguration {
            schema_version: 1,
            id: format!("provider-{provider_id}-default"),
            provider_id,
            enabled: true,
            default_model: match provider_id {
                ModelProviderId::Deepseek => "deepseek-v4-flash",
                ModelProviderId::Qwen => "qwen-plus",
                ModelProviderId::Kimi => "kimi-k2.6",
                ModelProviderId::Minimax => "MiniMax-M3",
                ModelProviderId::Ollama => "qwen3:8b",
            }
            .into(),
            credential_ref: format!("secret://providers/{provider_id}/default"),
            qwen: None,
            default_timeout_ms: 60_000,
            max_request_bytes: 16 * 1024 * 1024,
            updated_at: "2026-07-15T00:00:00Z".into(),
        }
    }

    fn model_request(provider_id: ModelProviderId) -> ModelExecutionRequest {
        ModelExecutionRequest {
            schema_version: 1,
            request_id: format!("request-{provider_id}"),
            configuration: configuration(provider_id),
            request: ModelRequest {
                model: None,
                messages: vec![ModelMessage {
                    role: ModelMessageRole::User,
                    content: "优化这段文字".into(),
                    name: None,
                    reasoning_content: None,
                    tool_call_id: None,
                    tool_calls: None,
                }],
                max_output_tokens: Some(512),
                temperature: None,
                top_p: None,
                stop: None,
                response_format: None,
                reasoning: None,
                tools: None,
                tool_choice: None,
            },
        }
    }

    fn host_with_secret(transport: Arc<dyn ModelTransport>) -> ModelExecutionHost {
        let secrets = Arc::new(MemorySecretStore::default());
        let reference = SecretReference::parse("secret://providers/deepseek/default").unwrap();
        secrets
            .put(&reference, SecretValue::new("host-only-key").unwrap())
            .unwrap();
        let reference = SecretReference::parse("secret://providers/minimax/default").unwrap();
        secrets
            .put(&reference, SecretValue::new("host-only-key").unwrap())
            .unwrap();
        ModelExecutionHost::with_transport(secrets, transport)
    }

    fn authorization_scope() -> ModelAuthorizationScope {
        ModelAuthorizationScope {
            project_id: "project-1".into(),
            base_commit_id: "commit-1".into(),
            operation_intent_id: "intent-1".into(),
            confirmed_context: ConfirmedContextPacket {
                id: "packet-1".into(),
                hash: format!("sha256:{}", "1".repeat(64)),
                payload_json: r#"{"content":"confirmed-context-secret-marker"}"#.into(),
            },
            target_block_id: "block-1".into(),
            target_block_revision: 0,
            target_block_hash: format!("sha256:{}", "2".repeat(64)),
        }
    }

    #[test]
    fn builds_only_fixed_official_endpoints_and_provider_dialects() {
        let mut deepseek = model_request(ModelProviderId::Deepseek);
        deepseek.request.reasoning = Some(ReasoningOptions {
            mode: ReasoningMode::Enabled,
            effort: Some(ReasoningEffort::Max),
            preserve: None,
        });
        deepseek.request.response_format = Some(ModelResponseFormat::JsonObject);
        let prepared = prepare_request(&deepseek).unwrap();
        assert_eq!(prepared.host(), "api.deepseek.com");
        assert_eq!(prepared.path(), "/chat/completions");
        let body: Value = serde_json::from_slice(prepared.body()).unwrap();
        assert_eq!(body["model"], "deepseek-v4-flash");
        assert_eq!(body["max_tokens"], 512);
        assert_eq!(body["thinking"], json!({ "type": "enabled" }));
        assert_eq!(body["reasoning_effort"], "max");
        assert_eq!(body["response_format"], json!({ "type": "json_object" }));

        let mut qwen = model_request(ModelProviderId::Qwen);
        qwen.configuration.qwen = Some(QwenProviderConfiguration {
            region: QwenDeploymentRegion::Germany,
            workspace_id: Some("workspace_123".into()),
        });
        qwen.request.reasoning = Some(ReasoningOptions {
            mode: ReasoningMode::Disabled,
            effort: None,
            preserve: Some(true),
        });
        let prepared = prepare_request(&qwen).unwrap();
        assert_eq!(
            prepared.host(),
            "workspace_123.eu-central-1.maas.aliyuncs.com"
        );
        assert_eq!(prepared.path(), "/compatible-mode/v1/chat/completions");
        let body: Value = serde_json::from_slice(prepared.body()).unwrap();
        assert_eq!(body["enable_thinking"], false);
        assert_eq!(body["preserve_thinking"], true);
        assert_eq!(body["max_completion_tokens"], 512);

        let mut kimi = model_request(ModelProviderId::Kimi);
        kimi.request.reasoning = Some(ReasoningOptions {
            mode: ReasoningMode::Enabled,
            effort: None,
            preserve: Some(true),
        });
        let prepared = prepare_request(&kimi).unwrap();
        assert_eq!(prepared.host(), "api.moonshot.cn");
        assert_eq!(prepared.path(), "/v1/chat/completions");
        let body: Value = serde_json::from_slice(prepared.body()).unwrap();
        assert_eq!(
            body["thinking"],
            json!({ "type": "enabled", "keep": "all" })
        );
        assert_eq!(body["max_tokens"], 512);

        let mut minimax = model_request(ModelProviderId::Minimax);
        minimax.request.reasoning = Some(ReasoningOptions {
            mode: ReasoningMode::Disabled,
            effort: None,
            preserve: None,
        });
        let prepared = prepare_request(&minimax).unwrap();
        assert_eq!(prepared.host(), "api.minimaxi.com");
        let body: Value = serde_json::from_slice(prepared.body()).unwrap();
        assert_eq!(body["reasoning_split"], true);
        assert_eq!(body["thinking"], json!({ "type": "disabled" }));
        assert_eq!(body["max_completion_tokens"], 512);

        let mut ollama = model_request(ModelProviderId::Ollama);
        ollama.request.reasoning = Some(ReasoningOptions {
            mode: ReasoningMode::Disabled,
            effort: None,
            preserve: None,
        });
        ollama.request.response_format = Some(ModelResponseFormat::JsonObject);
        let prepared = prepare_request(&ollama).unwrap();
        assert_eq!(prepared.host(), "127.0.0.1");
        assert_eq!(prepared.port(), 11_434);
        assert!(!prepared.use_tls());
        assert_eq!(prepared.path(), "/v1/chat/completions");
        let body: Value = serde_json::from_slice(prepared.body()).unwrap();
        assert_eq!(body["model"], "qwen3:8b");
        assert_eq!(body["max_tokens"], 512);
        assert_eq!(body["reasoning_effort"], "none");
        assert_eq!(body["response_format"], json!({ "type": "json_object" }));
    }

    #[test]
    fn resolves_secret_inside_host_and_streams_split_utf8_without_leaking_it() {
        let payload = concat!(
            "data: {\"id\":\"response-1\",\"model\":\"deepseek-v4-flash\",",
            "\"choices\":[{\"delta\":{\"reasoning_content\":\"思考\"},\"finish_reason\":null}]}\n\n",
            "data: {\"id\":\"response-1\",\"model\":\"deepseek-v4-flash\",",
            "\"choices\":[{\"delta\":{\"content\":\"修改后\"},\"finish_reason\":\"stop\"}],",
            "\"usage\":{\"prompt_tokens\":3,\"completion_tokens\":2,\"total_tokens\":5}}\n\n",
            "data: [DONE]\n\n"
        )
        .as_bytes()
        .to_vec();
        let split = payload
            .windows("思".len())
            .position(|window| window == "思".as_bytes())
            .unwrap()
            + 1;
        let transport = Arc::new(ScriptedTransport::sse(vec![
            payload[..split].to_vec(),
            payload[split..].to_vec(),
        ]));
        let host = host_with_secret(transport.clone());
        let mut events = Vec::new();
        let summary = host
            .execute_stream(model_request(ModelProviderId::Deepseek), |event| {
                events.push(event);
                Ok(())
            })
            .unwrap();
        assert!(transport.saw_expected_secret.load(Ordering::Relaxed));
        let captured = transport.captured.lock().unwrap().clone().unwrap();
        assert!(!String::from_utf8_lossy(captured.body()).contains("host-only-key"));
        assert_eq!(summary.content, "修改后");
        assert_eq!(summary.reasoning_content, "思考");
        assert_eq!(summary.finish_reason, ModelFinishReason::Stop);
        assert_eq!(summary.usage.as_ref().unwrap().total_tokens, 5);
        assert_eq!(events.len(), 5);
        let serialized = serde_json::to_string(&(summary, events)).unwrap();
        assert!(!serialized.contains("host-only-key"));
        assert!(!host.cancel("request-deepseek").cancelled);
    }

    #[test]
    fn authorizations_are_short_lived_scoped_and_single_use() {
        let transport = Arc::new(ScriptedTransport::sse(Vec::new()));
        let host = host_with_secret(transport);
        let request = model_request(ModelProviderId::Deepseek);
        let scope_debug = format!("{:?}", authorization_scope());
        assert!(!scope_debug.contains("confirmed-context-secret-marker"));
        assert!(scope_debug.contains("payload_bytes"));
        let authorization = host
            .authorize(request.clone(), authorization_scope())
            .unwrap();

        assert_eq!(authorization.schema_version, 1);
        assert_eq!(authorization.request_id, request.request_id);
        assert_eq!(authorization.provider_id, ModelProviderId::Deepseek);
        assert!(authorization.authorization_id.starts_with("model-auth-"));
        assert!(OffsetDateTime::parse(&authorization.expires_at, &Rfc3339).is_ok());
        assert_eq!(
            host.authorize(request.clone(), authorization_scope())
                .unwrap_err()
                .code(),
            "MODEL_REQUEST_ID_IN_USE"
        );
        assert_eq!(
            host.take_authorized_request(&authorization.authorization_id, "project-1", "commit-1")
                .unwrap(),
            request
        );
        assert_eq!(
            host.take_authorized_request(&authorization.authorization_id, "project-1", "commit-1")
                .unwrap_err()
                .code(),
            "MODEL_AUTHORIZATION_INVALID"
        );

        let stale = host
            .authorize(
                model_request(ModelProviderId::Deepseek),
                authorization_scope(),
            )
            .unwrap();
        assert_eq!(
            host.take_authorized_request(&stale.authorization_id, "project-1", "commit-2")
                .unwrap_err()
                .code(),
            "MODEL_AUTHORIZATION_STALE"
        );
        assert_eq!(
            host.take_authorized_request(&stale.authorization_id, "project-1", "commit-1")
                .unwrap_err()
                .code(),
            "MODEL_AUTHORIZATION_INVALID"
        );
    }

    #[test]
    fn expired_and_cancelled_authorizations_cannot_execute() {
        let transport = Arc::new(ScriptedTransport::sse(Vec::new()));
        let host = host_with_secret(transport);
        let expired = host
            .authorize(
                model_request(ModelProviderId::Deepseek),
                authorization_scope(),
            )
            .unwrap();
        host.pending
            .lock()
            .unwrap()
            .get_mut(&expired.authorization_id)
            .unwrap()
            .expires_at = OffsetDateTime::now_utc() - time::Duration::seconds(1);
        assert_eq!(
            host.take_authorized_request(&expired.authorization_id, "project-1", "commit-1")
                .unwrap_err()
                .code(),
            "MODEL_AUTHORIZATION_EXPIRED"
        );

        let cancelled = host
            .authorize(
                model_request(ModelProviderId::Deepseek),
                authorization_scope(),
            )
            .unwrap();
        assert!(host.cancel(&cancelled.request_id).cancelled);
        assert_eq!(
            host.take_authorized_request(&cancelled.authorization_id, "project-1", "commit-1")
                .unwrap_err()
                .code(),
            "MODEL_AUTHORIZATION_INVALID"
        );
    }

    #[test]
    fn normalizes_minimax_cumulative_content_and_reasoning_details() {
        let frames = concat!(
            "data: {\"id\":\"mm-1\",\"model\":\"MiniMax-M3\",\"choices\":[{",
            "\"delta\":{\"reasoning_details\":[{\"text\":\"思\"}],\"content\":\"你\"},",
            "\"finish_reason\":null}]}\n\n",
            "data: {\"id\":\"mm-1\",\"model\":\"MiniMax-M3\",\"choices\":[{",
            "\"delta\":{\"reasoning_details\":[{\"text\":\"思考\"}],\"content\":\"你好\"},",
            "\"finish_reason\":\"stop\"}]}\n\n",
            "data: [DONE]\n\n"
        );
        let transport = Arc::new(ScriptedTransport::sse(vec![frames.as_bytes().to_vec()]));
        let host = host_with_secret(transport);
        let mut deltas = Vec::new();
        let summary = host
            .execute_stream(model_request(ModelProviderId::Minimax), |event| {
                match &event {
                    ModelStreamEvent::TextDelta { text }
                    | ModelStreamEvent::ReasoningDelta { text } => deltas.push(text.clone()),
                    _ => {}
                }
                Ok(())
            })
            .unwrap();
        assert_eq!(deltas, ["思", "你", "考", "好"]);
        assert_eq!(summary.content, "你好");
        assert_eq!(summary.reasoning_content, "思考");
    }

    #[test]
    fn missing_secret_and_invalid_configuration_fail_before_network() {
        let transport = Arc::new(ScriptedTransport::sse(Vec::new()));
        let host = ModelExecutionHost::with_transport(
            Arc::new(MemorySecretStore::default()),
            transport.clone(),
        );
        let error = host
            .execute_stream(model_request(ModelProviderId::Deepseek), |_| Ok(()))
            .unwrap_err();
        assert_eq!(error.code(), "PROVIDER_CREDENTIAL_MISSING");
        assert_eq!(transport.calls.load(Ordering::Relaxed), 0);

        let mut cross_provider = model_request(ModelProviderId::Deepseek);
        cross_provider.configuration.credential_ref = "secret://providers/qwen/default".into();
        let error = host.execute_stream(cross_provider, |_| Ok(())).unwrap_err();
        assert_eq!(error.code(), "PROVIDER_CONFIGURATION");
        assert_eq!(transport.calls.load(Ordering::Relaxed), 0);

        let mut invalid = model_request(ModelProviderId::Deepseek);
        invalid.configuration.qwen = Some(QwenProviderConfiguration {
            region: QwenDeploymentRegion::China,
            workspace_id: None,
        });
        let error = host.execute_stream(invalid, |_| Ok(())).unwrap_err();
        assert_eq!(error.code(), "PROVIDER_CONFIGURATION");
        assert_eq!(transport.calls.load(Ordering::Relaxed), 0);

        let mut invalid = model_request(ModelProviderId::Qwen);
        invalid.configuration.qwen = Some(QwenProviderConfiguration {
            region: QwenDeploymentRegion::Japan,
            workspace_id: None,
        });
        assert_eq!(
            prepare_request(&invalid).unwrap_err().code(),
            "PROVIDER_CONFIGURATION"
        );
    }

    #[test]
    fn ollama_executes_on_fixed_loopback_without_resolving_a_secret() {
        let frames = concat!(
            "data: {\"id\":\"ollama-1\",\"model\":\"qwen3:8b\",\"choices\":[{",
            "\"delta\":{\"reasoning\":\"本地推理\",\"content\":\"本地结果\"},",
            "\"finish_reason\":\"stop\"}],",
            "\"usage\":{\"prompt_tokens\":4,\"completion_tokens\":2,\"total_tokens\":6}}\n\n",
            "data: [DONE]\n\n"
        );
        let transport = Arc::new(ScriptedTransport::sse(vec![frames.as_bytes().to_vec()]));
        let host = ModelExecutionHost::with_transport(
            Arc::new(MemorySecretStore::default()),
            transport.clone(),
        );

        let summary = host
            .execute_stream(model_request(ModelProviderId::Ollama), |_| Ok(()))
            .unwrap();

        assert_eq!(summary.provider_id, ModelProviderId::Ollama);
        assert_eq!(summary.model, "qwen3:8b");
        assert_eq!(summary.content, "本地结果");
        assert_eq!(summary.reasoning_content, "本地推理");
        assert_eq!(summary.usage.unwrap().total_tokens, 6);
        assert_eq!(transport.calls.load(Ordering::Relaxed), 1);
        assert!(!transport.saw_expected_secret.load(Ordering::Relaxed));
        let prepared = transport.captured.lock().unwrap().clone().unwrap();
        assert_eq!(prepared.host(), "127.0.0.1");
        assert_eq!(prepared.port(), 11_434);
        assert!(!prepared.use_tls());
    }

    #[test]
    fn discovers_and_validates_sorted_ollama_models_over_fixed_get() {
        let transport = Arc::new(ScriptedTransport {
            head: ModelHttpResponseHead {
                status: 200,
                headers: BTreeMap::from([("content-type".into(), "application/json".into())]),
            },
            chunks: vec![
                br#"{"data":[{"id":"zeta:latest","created":2,"owned_by":"library"},{"id":"alpha:8b","created":1}]}"#
                    .to_vec(),
            ],
            calls: AtomicUsize::new(0),
            saw_expected_secret: AtomicBool::new(false),
            captured: Mutex::new(None),
        });
        let host = ModelExecutionHost::with_transport(
            Arc::new(MemorySecretStore::default()),
            transport.clone(),
        );

        let response = host.list_ollama_models().unwrap();

        assert_eq!(response.schema_version, 1);
        assert_eq!(response.endpoint, "127.0.0.1:11434/v1");
        assert_eq!(
            response
                .models
                .iter()
                .map(|model| model.id.as_str())
                .collect::<Vec<_>>(),
            ["alpha:8b", "zeta:latest"]
        );
        assert_eq!(response.models[1].owned_by.as_deref(), Some("library"));
        let prepared = transport.captured.lock().unwrap().clone().unwrap();
        assert_eq!(prepared.method(), "GET");
        assert_eq!(prepared.host(), "127.0.0.1");
        assert_eq!(prepared.port(), 11_434);
        assert_eq!(prepared.path(), "/v1/models");
        assert!(prepared.body().is_empty());
        assert!(!prepared.use_tls());
        assert!(!transport.saw_expected_secret.load(Ordering::Relaxed));
    }

    #[test]
    fn normalizes_http_errors_retry_metadata_and_secret_redaction() {
        let transport = Arc::new(ScriptedTransport {
            head: ModelHttpResponseHead {
                status: 429,
                headers: BTreeMap::from([
                    ("content-type".into(), "application/json".into()),
                    ("retry-after".into(), "2".into()),
                    ("x-request-id".into(), "remote-host-only-key".into()),
                ]),
            },
            chunks: vec![
                br#"{"error":{"code":"rate_limit-host-only-key","message":"token host-only-key exceeded"}}"#
                    .to_vec(),
            ],
            calls: AtomicUsize::new(0),
            saw_expected_secret: AtomicBool::new(false),
            captured: Mutex::new(None),
        });
        let host = host_with_secret(transport);
        let error = host
            .execute_stream(model_request(ModelProviderId::Deepseek), |_| Ok(()))
            .unwrap_err();
        assert_eq!(error.code(), "PROVIDER_RATE_LIMIT");
        assert_eq!(error.status(), Some(429));
        assert_eq!(error.remote_code(), Some("rate_limit-[REDACTED]"));
        assert_eq!(error.remote_request_id(), Some("remote-[REDACTED]"));
        assert_eq!(error.retry_after_ms(), Some(2_000));
        assert!(error.retriable());
        assert!(!error.remote_code().unwrap().contains("host-only-key"));
        assert!(!error.remote_request_id().unwrap().contains("host-only-key"));
        assert!(!error.public_message().contains("host-only-key"));
        assert!(error.public_message().contains("[REDACTED]"));
    }

    #[test]
    fn redacts_secrets_before_truncating_provider_metadata() {
        let secret = "host-only-key";
        for (prefix, limit) in [
            ("m".repeat(995), 1_000),
            ("c".repeat(195), 200),
            ("r".repeat(507), 512),
        ] {
            let safe = bounded_redacted(&format!("{prefix}{secret}"), Some(secret), limit);
            assert_eq!(safe.chars().count(), limit);
            assert!(!safe.contains("host-"));
        }
    }

    #[test]
    fn cancellation_interrupts_an_active_transport_and_duplicate_ids_are_rejected() {
        let (started_send, started_receive) = mpsc::channel();
        let transport = Arc::new(BlockingTransport {
            started: Mutex::new(Some(started_send)),
        });
        let host = host_with_secret(transport);
        let worker_host = host.clone();
        let worker = std::thread::spawn(move || {
            worker_host.execute_stream(model_request(ModelProviderId::Deepseek), |_| Ok(()))
        });
        started_receive
            .recv_timeout(Duration::from_secs(2))
            .unwrap();

        let duplicate = host
            .execute_stream(model_request(ModelProviderId::Deepseek), |_| Ok(()))
            .unwrap_err();
        assert_eq!(duplicate.code(), "MODEL_REQUEST_ID_IN_USE");
        assert!(host.cancel("request-deepseek").cancelled);
        let cancelled = worker.join().unwrap().unwrap_err();
        assert_eq!(cancelled.code(), "PROVIDER_CANCELLED");
        assert!(!cancelled.retriable());
    }

    #[test]
    fn ready_callback_runs_only_after_the_request_is_cancellable() {
        let transport = Arc::new(BlockingTransport {
            started: Mutex::new(None),
        });
        let host = host_with_secret(transport);
        let error = host
            .execute_stream_after_ready(
                model_request(ModelProviderId::Deepseek),
                || assert!(host.cancel("request-deepseek").cancelled),
                |_| Ok(()),
            )
            .unwrap_err();
        assert_eq!(error.code(), "PROVIDER_CANCELLED");
    }

    #[test]
    fn malformed_sse_is_a_protocol_error() {
        let transport = Arc::new(ScriptedTransport::sse(vec![
            b"data: {not-json}\n\ndata: [DONE]\n\n".to_vec(),
        ]));
        let host = host_with_secret(transport);
        let error = host
            .execute_stream(model_request(ModelProviderId::Deepseek), |_| Ok(()))
            .unwrap_err();
        assert_eq!(error.code(), "PROVIDER_PROTOCOL");
    }
}
