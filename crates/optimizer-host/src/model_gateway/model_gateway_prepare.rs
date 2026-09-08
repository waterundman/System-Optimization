use serde_json::{Map, json};

use crate::TrustedModelEndpointCapabilities;

use super::*;
use super::model_gateway_transport::{ModelHttpMethod, PreparedModelRequest};

pub(crate) fn prepare_request(
    input: &ModelExecutionRequest,
    trusted_endpoint: Option<&TrustedModelEndpoint>,
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
    validate_model_request(
        input.configuration.provider_id,
        input.configuration.openai_compatible.as_ref(),
        &input.request,
    )?;
    let endpoint = provider_endpoint(&input.configuration, trusted_endpoint)?;
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
    let body = build_request_body(
        input.configuration.provider_id,
        input.configuration.openai_compatible.as_ref(),
        model,
        &input.request,
    )?;
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
        use_system_proxy: endpoint.use_system_proxy,
        method: ModelHttpMethod::Post,
        body,
        timeout_ms: input.configuration.default_timeout_ms,
        max_response_bytes: MAX_RESPONSE_BYTES,
    })
}

pub(crate) fn validate_request_id(request_id: &str) -> Result<(), ModelGatewayError> {
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

pub(crate) fn validate_authorization_id(authorization_id: &str) -> Result<(), ModelGatewayError> {
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

pub(crate) fn validate_configuration(
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
    let expected_credential_ref = if provider_id == ModelProviderId::OpenAICompatible {
        let compatible = configuration.openai_compatible.as_ref().ok_or_else(|| {
            configuration_error(
                provider_id,
                "openaiCompatible configuration is required for this provider",
            )
        })?;
        if compatible.endpoint_revision != 1 || !safe_endpoint_id(&compatible.endpoint_id) {
            return Err(configuration_error(
                provider_id,
                "openaiCompatible endpoint identity or revision is invalid",
            ));
        }
        format!(
            "secret://providers/openai-compatible/{}",
            compatible.endpoint_id
        )
    } else {
        format!("secret://providers/{provider_id}/default")
    };
    if credential_ref.as_str() != expected_credential_ref {
        return Err(configuration_error(
            provider_id,
            "credentialRef must match the Host-bound provider credential slot",
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
    if provider_id != ModelProviderId::OpenAICompatible && configuration.openai_compatible.is_some()
    {
        return Err(configuration_error(
            provider_id,
            "openaiCompatible configuration is only valid for the openai_compatible provider",
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

pub(crate) fn validate_model_request(
    provider_id: ModelProviderId,
    openai_compatible: Option<&OpenAICompatibleProviderConfiguration>,
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
    if provider_id == ModelProviderId::OpenAICompatible
        && request.response_format == Some(ModelResponseFormat::JsonObject)
        && openai_compatible.is_none_or(|configuration| !configuration.json_object)
    {
        return Err(invalid_request(
            provider_id,
            "the trusted endpoint has no declared json_object capability",
        ));
    }
    if provider_id == ModelProviderId::OpenAICompatible
        && (request.reasoning.is_some()
            || request.tools.is_some()
            || request.tool_choice.is_some()
            || request.messages.iter().any(|message| {
                message.role == ModelMessageRole::Tool
                    || message.reasoning_content.is_some()
                    || message.tool_call_id.is_some()
                    || message.tool_calls.is_some()
            }))
    {
        return Err(invalid_request(
            provider_id,
            "the trusted endpoint has no declared reasoning or tool-call capability",
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

pub(crate) fn valid_tool_name(value: &str) -> bool {
    !value.is_empty()
        && value.len() <= 64
        && value
            .bytes()
            .all(|byte| byte.is_ascii_alphanumeric() || matches!(byte, b'_' | b'-'))
}

pub(crate) fn safe_endpoint_id(value: &str) -> bool {
    value.starts_with("endpoint-")
        && value.len() == "endpoint-".len() + 32
        && value["endpoint-".len()..]
            .bytes()
            .all(|byte| byte.is_ascii_hexdigit() && !byte.is_ascii_uppercase())
}

struct ProviderEndpoint {
    host: String,
    path: String,
    port: u16,
    use_tls: bool,
    use_system_proxy: bool,
}

fn provider_endpoint(
    configuration: &ModelProviderConfiguration,
    trusted_endpoint: Option<&TrustedModelEndpoint>,
) -> Result<ProviderEndpoint, ModelGatewayError> {
    let endpoint = match configuration.provider_id {
        ModelProviderId::Deepseek => (
            "api.deepseek.com".to_string(),
            "/chat/completions".to_string(),
        ),
        ModelProviderId::Kimi => (
            "api.moonshot.cn".to_string(),
            "/v1/chat/completions".to_string(),
        ),
        ModelProviderId::Minimax => (
            "api.minimaxi.com".to_string(),
            "/v1/chat/completions".to_string(),
        ),
        ModelProviderId::Ollama => ("127.0.0.1".to_string(), "/v1/chat/completions".to_string()),
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
            (host, "/compatible-mode/v1/chat/completions".to_string())
        }
        ModelProviderId::OpenAICompatible => {
            let endpoint = trusted_endpoint.ok_or_else(|| {
                configuration_error(
                    ModelProviderId::OpenAICompatible,
                    "trusted endpoint is not registered",
                )
            })?;
            let compatible = configuration.openai_compatible.as_ref().ok_or_else(|| {
                configuration_error(
                    ModelProviderId::OpenAICompatible,
                    "openaiCompatible configuration is required",
                )
            })?;
            let expected_capabilities = TrustedModelEndpointCapabilities {
                json_object: compatible.json_object,
                stream_usage: compatible.stream_usage,
                max_output_token_field: compatible.max_output_token_field,
            };
            if endpoint.id != compatible.endpoint_id
                || endpoint.revision != compatible.endpoint_revision
                || endpoint.credential_ref != configuration.credential_ref
                || configuration.id != format!("provider-openai-compatible-{}", endpoint.id)
                || endpoint.capabilities != expected_capabilities
            {
                return Err(configuration_error(
                    ModelProviderId::OpenAICompatible,
                    "provider configuration does not match the current trusted endpoint record",
                ));
            }
            (endpoint.hostname.clone(), endpoint.chat_completions_path())
        }
    };
    Ok(ProviderEndpoint {
        host: endpoint.0,
        path: endpoint.1,
        port: if configuration.provider_id == ModelProviderId::Ollama {
            11_434
        } else {
            443
        },
        use_tls: configuration.provider_id != ModelProviderId::Ollama,
        use_system_proxy: !matches!(
            configuration.provider_id,
            ModelProviderId::Ollama | ModelProviderId::OpenAICompatible
        ),
    })
}

pub(crate) fn build_request_body(
    provider_id: ModelProviderId,
    openai_compatible: Option<&OpenAICompatibleProviderConfiguration>,
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
    ]);
    if provider_id != ModelProviderId::OpenAICompatible
        || openai_compatible.is_some_and(|configuration| configuration.stream_usage)
    {
        body.insert("stream_options".into(), json!({ "include_usage": true }));
    }
    if let Some(maximum) = request.max_output_tokens {
        let field = match provider_id {
            ModelProviderId::Deepseek | ModelProviderId::Kimi | ModelProviderId::Ollama => {
                "max_tokens"
            }
            ModelProviderId::Qwen | ModelProviderId::Minimax => "max_completion_tokens",
            ModelProviderId::OpenAICompatible => openai_compatible
                .map(|configuration| configuration.max_output_token_field.as_str())
                .unwrap_or("max_tokens"),
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
    if let Some(format) = request.response_format
        && !(provider_id == ModelProviderId::OpenAICompatible
            && format == ModelResponseFormat::Text)
    {
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

pub(crate) fn serialize_message(message: &ModelMessage) -> Value {
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

pub(crate) fn apply_reasoning_dialect(
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
        ModelProviderId::OpenAICompatible => {}
        ModelProviderId::Minimax => unreachable!("handled above"),
    }
}

pub(crate) fn invalid_request(provider_id: ModelProviderId, message: impl Into<String>) -> ModelGatewayError {
    ModelGatewayError::new("INVALID_MODEL_REQUEST", message).for_provider(provider_id)
}

pub(crate) fn configuration_error(
    provider_id: ModelProviderId,
    message: impl Into<String>,
) -> ModelGatewayError {
    ModelGatewayError::new("PROVIDER_CONFIGURATION", message).for_provider(provider_id)
}
