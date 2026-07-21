import { ProviderError } from "./errors.ts";
import type {
  ModelMessage,
  ModelRequest,
  ProviderProfile,
} from "./types.ts";

export interface BuiltChatRequest {
  readonly url: string;
  readonly model: string;
  readonly body: Readonly<Record<string, unknown>>;
}

export function buildChatRequest(
  profile: ProviderProfile,
  request: ModelRequest,
  stream: boolean,
): BuiltChatRequest {
  validateProfile(profile);
  validateRequest(profile, request);
  const model = request.model?.trim() || profile.defaultModel;
  const body: Record<string, unknown> = {
    model,
    messages: request.messages.map(serializeMessage),
    stream,
  };
  if (stream && profile.capabilities.usageInStream) {
    body.stream_options = { include_usage: true };
  }
  if (request.maxOutputTokens !== undefined) {
    body[profile.maxOutputTokenField] = request.maxOutputTokens;
  }
  if (request.temperature !== undefined) body.temperature = request.temperature;
  if (request.topP !== undefined) body.top_p = request.topP;
  if (request.stop !== undefined) body.stop = request.stop;
  if (
    request.responseFormat !== undefined
    && !(profile.dialect === "openai_compatible" && request.responseFormat === "text")
  ) {
    body.response_format = { type: request.responseFormat };
  }
  if (request.tools !== undefined) body.tools = request.tools;
  if (request.toolChoice !== undefined) body.tool_choice = request.toolChoice;
  applyReasoningDialect(profile, model, request, body);

  const baseUrl = profile.baseUrl.endsWith("/") ? profile.baseUrl : `${profile.baseUrl}/`;
  return {
    url: new URL("chat/completions", baseUrl).toString(),
    model,
    body,
  };
}

function serializeMessage(message: ModelMessage): Readonly<Record<string, unknown>> {
  const output: Record<string, unknown> = {
    role: message.role,
    content: message.content,
  };
  if (message.name !== undefined) output.name = message.name;
  if (message.reasoningContent !== undefined) {
    output.reasoning_content = message.reasoningContent;
  }
  if (message.toolCallId !== undefined) output.tool_call_id = message.toolCallId;
  if (message.toolCalls !== undefined) output.tool_calls = message.toolCalls;
  return output;
}

function applyReasoningDialect(
  profile: ProviderProfile,
  model: string,
  request: ModelRequest,
  body: Record<string, unknown>,
): void {
  const reasoning = request.reasoning;
  if (profile.dialect === "minimax") {
    body.reasoning_split = true;
    if (reasoning) {
      body.thinking = {
        type: reasoning.mode === "disabled" ? "disabled" : "adaptive",
      };
    }
    return;
  }
  if (!reasoning) return;

  if (profile.dialect === "qwen") {
    if (reasoning.mode !== "adaptive") {
      body.enable_thinking = reasoning.mode === "enabled";
    }
    if (reasoning.preserve !== undefined) body.preserve_thinking = reasoning.preserve;
    if (reasoning.effort !== undefined) body.reasoning_effort = reasoning.effort;
    return;
  }

  if (profile.dialect === "kimi") {
    if (model.toLowerCase().includes("k2.7-code") && reasoning.mode === "disabled") {
      throw invalidRequest(profile, "kimi-k2.7-code always has thinking enabled");
    }
    body.thinking = {
      type: reasoning.mode === "disabled" ? "disabled" : "enabled",
      ...(reasoning.preserve ? { keep: "all" } : {}),
    };
    return;
  }

  if (profile.dialect === "ollama") {
    if (reasoning.mode === "disabled") {
      body.reasoning_effort = "none";
    } else if (reasoning.effort !== undefined) {
      body.reasoning_effort = reasoning.effort;
    } else if (reasoning.mode === "enabled") {
      body.reasoning_effort = "medium";
    }
    return;
  }

  if (profile.dialect === "openai_compatible") return;

  if (reasoning.mode !== "adaptive") {
    body.thinking = { type: reasoning.mode };
  }
  if (reasoning.effort !== undefined) body.reasoning_effort = reasoning.effort;
}

function validateProfile(profile: ProviderProfile): void {
  let parsed: URL;
  try {
    parsed = new URL(profile.baseUrl);
  } catch {
    throw configurationError(profile, "Provider baseUrl is invalid");
  }
  const fixedOllamaEndpoint = profile.id === "ollama"
    && profile.locality === "local"
    && parsed.protocol === "http:"
    && parsed.hostname === "127.0.0.1"
    && parsed.port === "11434"
    && parsed.pathname === "/v1";
  if (
    (parsed.protocol !== "https:" && !fixedOllamaEndpoint)
    || parsed.username
    || parsed.password
    || parsed.search
    || parsed.hash
  ) {
    throw configurationError(
      profile,
      "Provider baseUrl must be HTTPS, except for the fixed Ollama loopback endpoint, and must not contain credentials, query or fragment",
    );
  }
}

function validateRequest(profile: ProviderProfile, request: ModelRequest): void {
  if (request.messages.length === 0) {
    throw invalidRequest(profile, "At least one message is required");
  }
  for (const [index, message] of request.messages.entries()) {
    if (typeof message.content !== "string") {
      throw invalidRequest(profile, `messages[${index}].content must be a string`);
    }
    if (message.name !== undefined && !message.name.trim()) {
      throw invalidRequest(profile, `messages[${index}].name must not be empty`);
    }
    if (message.role === "tool" && !message.toolCallId?.trim()) {
      throw invalidRequest(profile, `messages[${index}] tool message requires toolCallId`);
    }
    if (
      message.role !== "assistant"
      && (message.toolCalls !== undefined || message.reasoningContent !== undefined)
    ) {
      throw invalidRequest(
        profile,
        `messages[${index}] reasoningContent/toolCalls require the assistant role`,
      );
    }
  }
  if (
    request.maxOutputTokens !== undefined
    && (!Number.isInteger(request.maxOutputTokens) || request.maxOutputTokens < 1)
  ) {
    throw invalidRequest(profile, "maxOutputTokens must be a positive integer");
  }
  if (
    request.temperature !== undefined
    && (!Number.isFinite(request.temperature) || request.temperature < 0 || request.temperature > 2)
  ) {
    throw invalidRequest(profile, "temperature must be between 0 and 2");
  }
  if (
    request.topP !== undefined
    && (!Number.isFinite(request.topP) || request.topP < 0 || request.topP > 1)
  ) {
    throw invalidRequest(profile, "topP must be between 0 and 1");
  }
  const stops = typeof request.stop === "string" ? [request.stop] : request.stop;
  const maximumStops = profile.id === "kimi" ? 5 : 16;
  if (
    stops !== undefined
    && (
      stops.length === 0
      || stops.length > maximumStops
      || stops.some((stop) => typeof stop !== "string" || stop.length === 0)
    )
  ) {
    throw invalidRequest(profile, `stop must contain 1-${maximumStops} non-empty strings`);
  }
  if (request.responseFormat === "json_object" && !profile.capabilities.jsonObject) {
    throw invalidRequest(profile, `${profile.label} has no verified json_object capability`);
  }
  if (request.reasoning !== undefined && !profile.capabilities.reasoning) {
    throw invalidRequest(profile, `${profile.label} has no reasoning capability`);
  }
  if (
    profile.id === "openai_compatible"
    && (
      request.toolChoice !== undefined
      || request.messages.some((message) => message.role === "tool"
        || message.toolCallId !== undefined
        || message.toolCalls !== undefined
        || message.reasoningContent !== undefined)
    )
  ) {
    throw invalidRequest(profile, `${profile.label} has no declared reasoning or tool-call capability`);
  }
  if (request.tools !== undefined) {
    if (!profile.capabilities.toolCalls) {
      throw invalidRequest(profile, `${profile.label} has no tool-call capability`);
    }
    if (request.tools.length === 0 || request.tools.length > 128) {
      throw invalidRequest(profile, "tools must contain 1-128 definitions");
    }
    for (const tool of request.tools) {
      if (!/^[A-Za-z0-9_-]{1,64}$/.test(tool.function.name)) {
        throw invalidRequest(profile, `Invalid tool name: ${tool.function.name}`);
      }
    }
  } else if (request.toolChoice !== undefined && request.toolChoice !== "none") {
    throw invalidRequest(profile, "toolChoice requires tools");
  }
}

function invalidRequest(profile: ProviderProfile, message: string): ProviderError {
  return new ProviderError({
    kind: "invalid_request",
    providerId: profile.id,
    message,
  });
}

function configurationError(profile: ProviderProfile, message: string): ProviderError {
  return new ProviderError({
    kind: "configuration",
    providerId: profile.id,
    message,
  });
}
