use std::collections::BTreeMap;

use serde_json::Map;
use time::format_description::well_known::Rfc2822;

use super::*;
use super::model_gateway_transport::{ModelHttpResponseHead, ModelResponseSink};

pub(crate) struct CollectingResponseSink {
    head: Option<ModelHttpResponseHead>,
    body: Vec<u8>,
    maximum: usize,
}

impl CollectingResponseSink {
    pub(crate) fn new(maximum: usize) -> Self {
        Self {
            head: None,
            body: Vec::new(),
            maximum,
        }
    }

    pub(crate) fn finish(
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
                "provider model list exceeded the configured size limit",
            ));
        }
        self.body.extend_from_slice(chunk);
        Ok(())
    }
}

pub(crate) fn parse_ollama_models(body: &[u8]) -> Result<OllamaModelList, ModelGatewayError> {
    Ok(OllamaModelList {
        schema_version: REQUEST_SCHEMA_VERSION,
        endpoint: "127.0.0.1:11434/v1".into(),
        models: parse_model_entries(body, ModelProviderId::Ollama)?,
    })
}

pub(crate) fn parse_model_entries(
    body: &[u8],
    provider_id: ModelProviderId,
) -> Result<Vec<OllamaModelInfo>, ModelGatewayError> {
    let protocol_error = |message: String| {
        ModelGatewayError::new("PROVIDER_PROTOCOL", message).for_provider(provider_id)
    };
    let root: Value = serde_json::from_slice(body)
        .map_err(|_| protocol_error("provider model list is not valid JSON".into()))?;
    let models = root
        .as_object()
        .and_then(|value| value.get("data"))
        .and_then(Value::as_array)
        .ok_or_else(|| protocol_error("provider model list must contain a data array".into()))?;
    if models.len() > 10_000 {
        return Err(protocol_error(
            "provider model list contains too many entries".into(),
        ));
    }
    let mut normalized = BTreeMap::new();
    for (index, value) in models.iter().enumerate() {
        let model = value
            .as_object()
            .ok_or_else(|| protocol_error(format!("data[{index}] must be an object")))?;
        let id = model
            .get("id")
            .and_then(Value::as_str)
            .filter(|id| !id.is_empty() && id.len() <= 256 && !id.chars().any(char::is_control))
            .ok_or_else(|| protocol_error(format!("data[{index}].id must be a safe model ID")))?
            .to_string();
        let created = match model.get("created") {
            Some(value) => Some(value.as_u64().ok_or_else(|| {
                protocol_error(format!("data[{index}].created must be an integer"))
            })?),
            None => None,
        };
        let owned_by = match model.get("owned_by") {
            Some(value) => {
                let value = value
                    .as_str()
                    .filter(|value| value.len() <= 128)
                    .ok_or_else(|| {
                        protocol_error(format!("data[{index}].owned_by must be a short string"))
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
            return Err(protocol_error(
                "provider model list contains duplicate IDs".into(),
            ));
        }
    }
    Ok(normalized.into_values().collect())
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub(crate) enum StreamContentMode {
    Delta,
    Cumulative,
}

pub(crate) struct StreamingGatewaySink<'a> {
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
    pub(crate) fn new(
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

    pub(crate) fn finish(mut self) -> Result<ModelExecutionSummary, ModelGatewayError> {
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

pub(crate) fn provider_http_error(
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

pub(crate) fn bounded_redacted(value: &str, secret: Option<&str>, max_chars: usize) -> String {
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
