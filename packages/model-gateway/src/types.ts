import type { ModelProviderId } from "../../protocol/src/index.ts";

export type ProviderId = ModelProviderId;
export type ProviderDialect = ProviderId;
export type MessageRole = "system" | "user" | "assistant" | "tool";
export type FinishReason =
  | "stop"
  | "length"
  | "content_filter"
  | "tool_calls"
  | "insufficient_system_resource"
  | "unknown";

export interface ModelToolCall {
  readonly id: string;
  readonly type: "function";
  readonly function: {
    readonly name: string;
    readonly arguments: string;
  };
}

export interface ModelMessage {
  readonly role: MessageRole;
  readonly content: string;
  readonly name?: string;
  readonly reasoningContent?: string;
  readonly toolCallId?: string;
  readonly toolCalls?: readonly ModelToolCall[];
}

export interface ModelToolDefinition {
  readonly type: "function";
  readonly function: {
    readonly name: string;
    readonly description?: string;
    readonly parameters?: Readonly<Record<string, unknown>>;
    readonly strict?: boolean;
  };
}

export interface ReasoningOptions {
  readonly mode: "enabled" | "disabled" | "adaptive";
  readonly effort?: "low" | "medium" | "high" | "max";
  readonly preserve?: boolean;
}

export interface ModelRequest {
  readonly model?: string;
  readonly messages: readonly ModelMessage[];
  readonly maxOutputTokens?: number;
  readonly temperature?: number;
  readonly topP?: number;
  readonly stop?: string | readonly string[];
  readonly responseFormat?: "text" | "json_object";
  readonly reasoning?: ReasoningOptions;
  readonly tools?: readonly ModelToolDefinition[];
  readonly toolChoice?: "none" | "auto" | "required";
}

export interface ModelUsage {
  readonly inputTokens: number;
  readonly outputTokens: number;
  readonly totalTokens: number;
  readonly cachedInputTokens?: number;
  readonly reasoningTokens?: number;
}

export interface ModelCompletion {
  readonly id: string;
  readonly providerId: ProviderId;
  readonly model: string;
  readonly content: string;
  readonly reasoningContent?: string;
  readonly toolCalls: readonly ModelToolCall[];
  readonly finishReason: FinishReason;
  readonly usage?: ModelUsage;
}

export type ModelStreamEvent =
  | {
      readonly type: "start";
      readonly id: string;
      readonly providerId: ProviderId;
      readonly model: string;
    }
  | { readonly type: "text_delta"; readonly text: string }
  | { readonly type: "reasoning_delta"; readonly text: string }
  | {
      readonly type: "tool_call_delta";
      readonly index: number;
      readonly id?: string;
      readonly name?: string;
      readonly arguments?: string;
    }
  | { readonly type: "usage"; readonly usage: ModelUsage }
  | { readonly type: "finish"; readonly reason: FinishReason };

export interface ProviderCapabilities {
  readonly streaming: boolean;
  readonly reasoning: boolean;
  readonly jsonObject: boolean;
  readonly toolCalls: boolean;
  readonly usageInStream: boolean;
}

export interface ProviderProfile {
  readonly id: ProviderId;
  readonly label: string;
  readonly dialect: ProviderDialect;
  readonly baseUrl: string;
  readonly apiKeyEnvironmentVariable: string;
  readonly defaultModel: string;
  readonly knownModels: readonly string[];
  readonly maxOutputTokenField: "max_tokens" | "max_completion_tokens";
  readonly streamContentMode: "delta" | "cumulative";
  readonly capabilities: ProviderCapabilities;
  readonly documentationUrl: string;
  readonly verifiedAt: string;
}

export interface HttpHeadersLike {
  get(name: string): string | null;
}

export interface ReadableStreamReaderLike {
  read(): Promise<{ readonly done: boolean; readonly value?: Uint8Array }>;
  cancel(reason?: unknown): Promise<void>;
  releaseLock(): void;
}

export interface ReadableByteStreamLike {
  getReader(): ReadableStreamReaderLike;
}

export interface HttpResponseLike {
  readonly ok: boolean;
  readonly status: number;
  readonly headers: HttpHeadersLike;
  readonly body: ReadableByteStreamLike | null;
  json(): Promise<unknown>;
}

export interface HttpRequestInit {
  readonly method: "POST";
  readonly headers: Readonly<Record<string, string>>;
  readonly body: string;
  readonly signal: AbortSignal;
}

export type FetchLike = (
  input: string,
  init: HttpRequestInit,
) => Promise<HttpResponseLike>;

export interface TimerPort {
  setTimeout(callback: () => void, milliseconds: number): unknown;
  clearTimeout(handle: unknown): void;
}

export interface ModelGatewayOptions {
  readonly signal?: AbortSignal;
  readonly timeoutMs?: number;
}

export interface ModelGatewayConfig {
  readonly profile: ProviderProfile;
  readonly apiKey: string;
  readonly fetch?: FetchLike;
  readonly timer?: TimerPort;
  readonly defaultTimeoutMs?: number;
  readonly maxRequestBytes?: number;
}

export interface ModelProvider {
  readonly profile: ProviderProfile;
  complete(
    request: ModelRequest,
    options?: ModelGatewayOptions,
  ): Promise<ModelCompletion>;
  stream(
    request: ModelRequest,
    options?: ModelGatewayOptions,
  ): AsyncIterable<ModelStreamEvent>;
}
