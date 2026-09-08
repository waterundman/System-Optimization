use std::collections::HashMap;
use std::fmt;
use std::sync::{Arc, Mutex, mpsc};
use std::time::Duration;

use serde::{Deserialize, Serialize};
use serde_json::Value;
use time::OffsetDateTime;
use time::format_description::well_known::Rfc3339;
use uuid::Uuid;

use crate::confirmed_context::ConfirmedContextPacket;
use crate::{
    OpenAICompatibleMaxOutputTokenField, SecretReference, SecretStore, SecretStoreError,
    SecretValue, TrustedModelEndpoint, TrustedModelEndpointRegistry,
};

const REQUEST_SCHEMA_VERSION: u32 = 1;
const MAX_TIMEOUT_MS: u32 = 3_600_000;
const MAX_REQUEST_BYTES: u32 = 67_108_864;
const MAX_RESPONSE_BYTES: usize = 67_108_864;
const MAX_ERROR_BYTES: usize = 65_536;
const MAX_SSE_EVENT_BYTES: usize = 4 * 1024 * 1024;
const MAX_OUTPUT_BYTES: usize = 32 * 1024 * 1024;
const MODEL_DISCOVERY_TIMEOUT_MS: u32 = 3_000;
const MODEL_DISCOVERY_MAX_BYTES: usize = 1024 * 1024;
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
    #[serde(rename = "openai_compatible")]
    OpenAICompatible,
}

impl ModelProviderId {
    pub fn as_str(self) -> &'static str {
        match self {
            Self::Deepseek => "deepseek",
            Self::Qwen => "qwen",
            Self::Kimi => "kimi",
            Self::Minimax => "minimax",
            Self::Ollama => "ollama",
            Self::OpenAICompatible => "openai_compatible",
        }
    }

    fn label(self) -> &'static str {
        match self {
            Self::Deepseek => "DeepSeek",
            Self::Qwen => "Qwen",
            Self::Kimi => "Kimi",
            Self::Minimax => "MiniMax",
            Self::Ollama => "Ollama",
            Self::OpenAICompatible => "OpenAI-compatible",
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
pub struct OpenAICompatibleProviderConfiguration {
    pub endpoint_id: String,
    pub endpoint_revision: u64,
    pub json_object: bool,
    pub stream_usage: bool,
    pub max_output_token_field: OpenAICompatibleMaxOutputTokenField,
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
    pub openai_compatible: Option<OpenAICompatibleProviderConfiguration>,
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
pub struct OpenAICompatibleModelList {
    pub schema_version: u32,
    pub endpoint_id: String,
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

mod model_gateway_prepare;
mod model_gateway_stream;
mod model_gateway_transport;

pub use model_gateway_transport::{
    ModelCancellation, ModelHttpResponseHead, ModelResponseSink, ModelTransport, PreparedModelRequest,
};
use model_gateway_prepare::{configuration_error, prepare_request, validate_authorization_id};
use model_gateway_stream::{
    CollectingResponseSink, StreamContentMode, StreamingGatewaySink, parse_model_entries,
    parse_ollama_models, provider_http_error,
};
use model_gateway_transport::{ActiveRequestGuard, ModelHttpMethod};

#[derive(Clone)]
pub struct ModelExecutionHost {
    secrets: Arc<dyn SecretStore>,
    trusted_endpoints: Arc<Mutex<TrustedModelEndpointRegistry>>,
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
        Self::with_transport_and_endpoints(
            secrets,
            Arc::new(super::model_transport::NativeModelTransport::new()),
            Arc::new(Mutex::new(TrustedModelEndpointRegistry::memory())),
        )
    }

    pub fn with_trusted_endpoints(
        secrets: Arc<dyn SecretStore>,
        trusted_endpoints: Arc<Mutex<TrustedModelEndpointRegistry>>,
    ) -> Self {
        Self::with_transport_and_endpoints(
            secrets,
            Arc::new(super::model_transport::NativeModelTransport::new()),
            trusted_endpoints,
        )
    }

    pub fn with_transport(
        secrets: Arc<dyn SecretStore>,
        transport: Arc<dyn ModelTransport>,
    ) -> Self {
        Self::with_transport_and_endpoints(
            secrets,
            transport,
            Arc::new(Mutex::new(TrustedModelEndpointRegistry::memory())),
        )
    }

    pub fn with_transport_and_endpoints(
        secrets: Arc<dyn SecretStore>,
        transport: Arc<dyn ModelTransport>,
        trusted_endpoints: Arc<Mutex<TrustedModelEndpointRegistry>>,
    ) -> Self {
        Self {
            secrets,
            trusted_endpoints,
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
        let prepared = self.prepare_request(&input)?;
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
            expires_at: expires_at.format(&Rfc3339).map_err(|error| {
                ModelGatewayError::new(
                    "MODEL_AUTHORIZATION_UNAVAILABLE",
                    format!("failed to format authorization expiry timestamp: {error}"),
                )
            })?,
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
            use_system_proxy: false,
            method: ModelHttpMethod::Get,
            body: Vec::new(),
            timeout_ms: MODEL_DISCOVERY_TIMEOUT_MS,
            max_response_bytes: MODEL_DISCOVERY_MAX_BYTES,
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

    pub fn list_openai_compatible_models(
        &self,
        endpoint_id: &str,
    ) -> Result<OpenAICompatibleModelList, ModelGatewayError> {
        let endpoint = self.trusted_endpoint(endpoint_id)?;
        let reference = SecretReference::parse(endpoint.credential_ref.clone())?;
        let secret = self.secrets.resolve(&reference)?.ok_or_else(|| {
            ModelGatewayError::new(
                "PROVIDER_CREDENTIAL_MISSING",
                format!("No credential is stored for {reference}"),
            )
            .for_provider(ModelProviderId::OpenAICompatible)
        })?;
        let prepared = PreparedModelRequest {
            request_id: format!("openai-compatible-model-list-{endpoint_id}"),
            provider_id: ModelProviderId::OpenAICompatible,
            host: endpoint.hostname.clone(),
            path: endpoint.models_path(),
            port: 443,
            use_tls: true,
            use_system_proxy: false,
            method: ModelHttpMethod::Get,
            body: Vec::new(),
            timeout_ms: MODEL_DISCOVERY_TIMEOUT_MS,
            max_response_bytes: MODEL_DISCOVERY_MAX_BYTES,
        };
        let control = ModelCancellation::new();
        let mut sink = CollectingResponseSink::new(prepared.max_response_bytes());
        self.transport
            .execute(&prepared, Some(&secret), &control, &mut sink)
            .map_err(|error| error.for_provider(ModelProviderId::OpenAICompatible))?;
        let (head, body) = sink.finish(ModelProviderId::OpenAICompatible)?;
        if !(200..300).contains(&head.status) {
            return Err(provider_http_error(
                ModelProviderId::OpenAICompatible,
                &head,
                &body,
                Some(secret.expose_secret()),
            ));
        }
        Ok(OpenAICompatibleModelList {
            schema_version: REQUEST_SCHEMA_VERSION,
            endpoint_id: endpoint.id,
            endpoint: endpoint.base_url,
            models: parse_model_entries(&body, ModelProviderId::OpenAICompatible)?,
        })
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
        let prepared = self.prepare_request(&input)?;
        let provider_id = prepared.provider_id;
        let secret = if provider_id == ModelProviderId::Ollama {
            None
        } else {
            // T1 hardening: never resolve the API key from the
            // WebView-supplied `configuration.credential_ref`. Derive the
            // reference strictly from the host-bound provider slot so a key
            // registered for one endpoint cannot be borrowed against another
            // host. `prepare_request` above has already verified the request
            // is destined for this slot's canonical hostname.
            let reference = self.host_bound_credential_reference(&input)?;
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

    fn prepare_request(
        &self,
        input: &ModelExecutionRequest,
    ) -> Result<PreparedModelRequest, ModelGatewayError> {
        let endpoint = if input.configuration.provider_id == ModelProviderId::OpenAICompatible {
            let compatible = input
                .configuration
                .openai_compatible
                .as_ref()
                .ok_or_else(|| {
                    configuration_error(
                        ModelProviderId::OpenAICompatible,
                        "openaiCompatible configuration is required",
                    )
                })?;
            Some(self.trusted_endpoint(&compatible.endpoint_id)?)
        } else {
            None
        };
        prepare_request(input, endpoint.as_ref())
    }

    fn trusted_endpoint(
        &self,
        endpoint_id: &str,
    ) -> Result<TrustedModelEndpoint, ModelGatewayError> {
        self.trusted_endpoints
            .lock()
            .map_err(|_| {
                ModelGatewayError::new(
                    "TRUSTED_MODEL_ENDPOINTS_UNAVAILABLE",
                    "trusted model endpoint state is unavailable",
                )
            })?
            .get(endpoint_id)
            .map_err(|error| {
                ModelGatewayError::new(error.code(), error.public_message())
                    .for_provider(ModelProviderId::OpenAICompatible)
            })
    }

    /// T1 hardening: derive the secret slot reference for a model request
    /// strictly from the host-bound provider configuration, ignoring the
    /// WebView-supplied `credential_ref`. For `openai_compatible` requests
    /// the key is resolved from the registered endpoint record, so a key
    /// bound to endpoint A can never be presented against endpoint B (the
    /// request destination host is always `endpoint.hostname`, see
    /// `provider_endpoint`).
    fn host_bound_credential_reference(
        &self,
        input: &ModelExecutionRequest,
    ) -> Result<SecretReference, ModelGatewayError> {
        let provider_id = input.configuration.provider_id;
        let reference = if provider_id == ModelProviderId::OpenAICompatible {
            let compatible = input
                .configuration
                .openai_compatible
                .as_ref()
                .ok_or_else(|| {
                    configuration_error(
                        provider_id,
                        "openaiCompatible configuration is required",
                    )
                })?;
            let endpoint = self.trusted_endpoint(&compatible.endpoint_id)?;
            SecretReference::parse(endpoint.credential_ref.clone())?
        } else {
            SecretReference::parse(format!("secret://providers/{provider_id}/default"))?
        };
        Ok(reference)
    }

    pub fn cancel(&self, request_id: impl Into<String>) -> CancelModelRequestResponse {
        let request_id = request_id.into();
        // Cancellation is best-effort: the response shape is a plain value
        // (not a Result) to keep the IPC surface stable. A poisoned lock —
        // only reachable if a previous holder panicked — is logged and
        // treated as a no-op rather than silently swallowed.
        let active_cancelled = match self.active.lock() {
            Ok(active) => active
                .get(&request_id)
                .cloned()
                .is_some_and(|control| control.cancel()),
            Err(_) => {
                eprintln!(
                    "[optimizer-host] model_gateway: cancel lost active state (poisoned mutex)"
                );
                false
            }
        };
        let pending_cancelled = match self.pending.lock() {
            Ok(mut pending) => pending
                .iter()
                .find_map(|(id, authorization)| {
                    (authorization.input.request_id == request_id).then(|| id.clone())
                })
                .is_some_and(|authorization_id| pending.remove(&authorization_id).is_some()),
            Err(_) => {
                eprintln!(
                    "[optimizer-host] model_gateway: cancel lost pending state (poisoned mutex)"
                );
                false
            }
        };
        CancelModelRequestResponse {
            schema_version: REQUEST_SCHEMA_VERSION,
            request_id,
            cancelled: active_cancelled || pending_cancelled,
        }
    }

    pub fn cancel_all(&self) -> usize {
        // Best-effort bulk cancellation (used at teardown); poisoned-lock
        // failures are logged and counted as nothing cancelled.
        let controls = match self.active.lock() {
            Ok(active) => active.values().cloned().collect::<Vec<_>>(),
            Err(_) => {
                eprintln!(
                    "[optimizer-host] model_gateway: cancel_all lost active state (poisoned mutex)"
                );
                Vec::new()
            }
        };
        let active = controls
            .into_iter()
            .filter(|control| control.cancel())
            .count();
        let pending = match self.pending.lock() {
            Ok(mut pending) => {
                let count = pending.len();
                pending.clear();
                count
            }
            Err(_) => {
                eprintln!(
                    "[optimizer-host] model_gateway: cancel_all lost pending state (poisoned mutex)"
                );
                0
            }
        };
        active + pending
    }
}
pub use super::model_transport::NativeModelTransport;

#[cfg(test)]
mod tests {
    use std::collections::BTreeMap;
    use std::sync::atomic::{AtomicBool, AtomicUsize, Ordering};

    use crate::MemorySecretStore;
    use crate::TrustedModelEndpointCapabilities;
    use serde_json::json;

    use super::model_gateway_stream::bounded_redacted;
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
                ModelProviderId::OpenAICompatible => "compatible-model",
            }
            .into(),
            credential_ref: format!("secret://providers/{provider_id}/default"),
            qwen: None,
            openai_compatible: None,
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

    fn trusted_compatible_registry() -> (TrustedModelEndpointRegistry, TrustedModelEndpoint) {
        let mut registry = TrustedModelEndpointRegistry::memory();
        let endpoint = registry
            .register(crate::RegisterTrustedModelEndpoint {
                schema_version: 1,
                label: "Acme Gateway".into(),
                hostname: "api.acme.ai".into(),
                base_path: "/openai/v1".into(),
                confirmed_origin: "https://api.acme.ai".into(),
                capabilities: TrustedModelEndpointCapabilities::default(),
            })
            .unwrap();
        (registry, endpoint)
    }

    fn compatible_request(endpoint: &TrustedModelEndpoint) -> ModelExecutionRequest {
        let mut request = model_request(ModelProviderId::OpenAICompatible);
        request.configuration.id = format!("provider-openai-compatible-{}", endpoint.id);
        request.configuration.credential_ref = endpoint.credential_ref.clone();
        request.configuration.openai_compatible = Some(OpenAICompatibleProviderConfiguration {
            endpoint_id: endpoint.id.clone(),
            endpoint_revision: endpoint.revision,
            json_object: endpoint.capabilities.json_object,
            stream_usage: endpoint.capabilities.stream_usage,
            max_output_token_field: endpoint.capabilities.max_output_token_field,
        });
        request.request.response_format = Some(ModelResponseFormat::Text);
        request
    }

    fn host_with_compatible_endpoint(
        registry: TrustedModelEndpointRegistry,
        endpoint: &TrustedModelEndpoint,
        transport: Arc<dyn ModelTransport>,
    ) -> ModelExecutionHost {
        let registry = Arc::new(Mutex::new(registry));
        let secrets = Arc::new(MemorySecretStore::default());
        let reference = SecretReference::parse(endpoint.credential_ref.clone()).unwrap();
        secrets
            .put(&reference, SecretValue::new("host-only-key").unwrap())
            .unwrap();
        ModelExecutionHost::with_transport_and_endpoints(secrets, transport, registry)
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
        let prepared = prepare_request(&deepseek, None).unwrap();
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
        let prepared = prepare_request(&qwen, None).unwrap();
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
        let prepared = prepare_request(&kimi, None).unwrap();
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
        let prepared = prepare_request(&minimax, None).unwrap();
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
        let prepared = prepare_request(&ollama, None).unwrap();
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
            prepare_request(&invalid, None).unwrap_err().code(),
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
    fn compatible_execution_resolves_only_the_registered_endpoint_and_conservative_dialect() {
        let frames = concat!(
            "data: {\"id\":\"compatible-1\",\"model\":\"compatible-model\",",
            "\"choices\":[{\"delta\":{\"content\":\"trusted result\"},\"finish_reason\":\"stop\"}]}\n\n",
            "data: [DONE]\n\n"
        );
        let transport = Arc::new(ScriptedTransport::sse(vec![frames.as_bytes().to_vec()]));
        let (registry, endpoint) = trusted_compatible_registry();
        let host = host_with_compatible_endpoint(registry, &endpoint, transport.clone());
        let request = compatible_request(&endpoint);

        let summary = host.execute_stream(request.clone(), |_| Ok(())).unwrap();

        assert_eq!(summary.provider_id, ModelProviderId::OpenAICompatible);
        assert_eq!(summary.content, "trusted result");
        assert!(transport.saw_expected_secret.load(Ordering::Relaxed));
        let prepared = transport.captured.lock().unwrap().clone().unwrap();
        assert_eq!(prepared.host(), "api.acme.ai");
        assert_eq!(prepared.path(), "/openai/v1/chat/completions");
        assert_eq!(prepared.port(), 443);
        assert!(prepared.use_tls());
        assert!(!prepared.use_system_proxy());
        let body: Value = serde_json::from_slice(prepared.body()).unwrap();
        assert_eq!(body["max_tokens"], 512);
        assert!(body.get("stream_options").is_none());
        assert!(body.get("response_format").is_none());
        assert!(body.get("thinking").is_none());
        assert!(body.get("reasoning_effort").is_none());

        let mut unsupported = request.clone();
        unsupported.request_id = "request-compatible-reasoning".into();
        unsupported.request.reasoning = Some(ReasoningOptions {
            mode: ReasoningMode::Enabled,
            effort: Some(ReasoningEffort::High),
            preserve: None,
        });
        assert_eq!(
            host.execute_stream(unsupported, |_| Ok(()))
                .unwrap_err()
                .code(),
            "INVALID_MODEL_REQUEST"
        );

        let mut tampered = request;
        tampered.request_id = "request-compatible-tampered".into();
        tampered
            .configuration
            .openai_compatible
            .as_mut()
            .unwrap()
            .stream_usage = true;
        assert_eq!(
            host.execute_stream(tampered, |_| Ok(()))
                .unwrap_err()
                .code(),
            "PROVIDER_CONFIGURATION"
        );
        assert_eq!(transport.calls.load(Ordering::Relaxed), 1);

        let mut forged_audit_identity = compatible_request(&endpoint);
        forged_audit_identity.request_id = "request-compatible-forged-config".into();
        forged_audit_identity.configuration.id = "provider-forged".into();
        assert_eq!(
            host.execute_stream(forged_audit_identity, |_| Ok(()))
                .unwrap_err()
                .code(),
            "PROVIDER_CONFIGURATION"
        );
        assert_eq!(transport.calls.load(Ordering::Relaxed), 1);
    }

    #[test]
    fn cross_endpoint_credential_borrow_is_rejected_before_transport() {
        let frames = concat!(
            "data: {\"id\":\"compatible-1\",\"model\":\"compatible-model\",",
            "\"choices\":[{\"delta\":{\"content\":\"trusted result\"},\"finish_reason\":\"stop\"}]}\n\n",
            "data: [DONE]\n\n"
        );
        let transport = Arc::new(ScriptedTransport::sse(vec![frames.as_bytes().to_vec()]));
        let (registry, endpoint) = trusted_compatible_registry();
        let host = host_with_compatible_endpoint(registry, &endpoint, transport.clone());
        let mut request = compatible_request(&endpoint);
        // T1: attacker points at endpoint A but tries to borrow endpoint B's
        // secret slot. `validate_model_request`/`provider_endpoint` must
        // reject this before anything is sent over the network.
        request.request_id = "request-compatible-borrow".into();
        request.configuration.credential_ref =
            "secret://providers/openai-compatible/endpoint-evil".into();
        assert_eq!(
            host.execute_stream(request, |_| Ok(()))
                .unwrap_err()
                .code(),
            "PROVIDER_CONFIGURATION"
        );
        assert_eq!(transport.calls.load(Ordering::Relaxed), 0);
    }

    #[test]
    fn builtin_provider_cannot_borrow_another_slots_key() {
        let frames = concat!(
            "data: {\"id\":\"kimi-1\",\"model\":\"kimi-k2.6\",",
            "\"choices\":[{\"delta\":{\"content\":\"ok\"},\"finish_reason\":\"stop\"}]}\n\n",
            "data: [DONE]\n\n"
        );
        let transport = Arc::new(ScriptedTransport::sse(vec![frames.as_bytes().to_vec()]));
        let host = host_with_secret(transport.clone());
        let mut request = model_request(ModelProviderId::Kimi);
        request.request.response_format = Some(ModelResponseFormat::Text);
        // T1: a Kimi request may only present the `secret://providers/kimi/default`
        // slot; borrowing the Deepseek slot must be rejected by the host.
        request.configuration.credential_ref = "secret://providers/deepseek/default".into();
        assert_eq!(
            host.execute_stream(request, |_| Ok(()))
                .unwrap_err()
                .code(),
            "PROVIDER_CONFIGURATION"
        );
        assert_eq!(transport.calls.load(Ordering::Relaxed), 0);
    }

    #[test]
    fn host_bound_credential_reference_ignores_webview_supplied_ref() {
        let transport = Arc::new(ScriptedTransport::sse(vec![]));
        let (registry, endpoint) = trusted_compatible_registry();
        let host = host_with_compatible_endpoint(registry, &endpoint, transport);
        let mut request = compatible_request(&endpoint);
        // Even if the WebView-supplied credential_ref is tampered with, the
        // host resolves the key strictly from the registered endpoint slot.
        request.configuration.credential_ref =
            "secret://providers/openai-compatible/endpoint-evil".into();
        let reference = host.host_bound_credential_reference(&request).unwrap();
        assert_eq!(reference.as_str(), endpoint.credential_ref);
    }

    #[test]
    fn compatible_models_probe_is_bounded_and_bound_to_the_endpoint_secret() {
        let transport = Arc::new(ScriptedTransport {
            head: ModelHttpResponseHead {
                status: 200,
                headers: BTreeMap::from([("content-type".into(), "application/json".into())]),
            },
            chunks: vec![
                br#"{"data":[{"id":"writer-z"},{"id":"writer-a","owned_by":"acme"}]}"#.to_vec(),
            ],
            calls: AtomicUsize::new(0),
            saw_expected_secret: AtomicBool::new(false),
            captured: Mutex::new(None),
        });
        let (registry, endpoint) = trusted_compatible_registry();
        let host = host_with_compatible_endpoint(registry, &endpoint, transport.clone());

        let response = host.list_openai_compatible_models(&endpoint.id).unwrap();

        assert_eq!(response.endpoint_id, endpoint.id);
        assert_eq!(response.endpoint, "https://api.acme.ai/openai/v1");
        assert_eq!(
            response
                .models
                .iter()
                .map(|model| model.id.as_str())
                .collect::<Vec<_>>(),
            ["writer-a", "writer-z"]
        );
        assert!(transport.saw_expected_secret.load(Ordering::Relaxed));
        let prepared = transport.captured.lock().unwrap().clone().unwrap();
        assert_eq!(prepared.method(), "GET");
        assert_eq!(prepared.path(), "/openai/v1/models");
        assert!(prepared.body().is_empty());
        assert!(!prepared.use_system_proxy());
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
