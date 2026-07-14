import { ProviderError, type ProviderErrorKind } from "./errors.ts";
import { buildChatRequest } from "./request.ts";
import {
  ResponseProtocolError,
  parseCompletionResponse,
  parseStreamChunk,
} from "./response.ts";
import { SseProtocolError, parseServerSentEvents } from "./sse.ts";
import type {
  FetchLike,
  HttpResponseLike,
  ModelCompletion,
  ModelGatewayConfig,
  ModelGatewayOptions,
  ModelRequest,
  ModelStreamEvent,
  ProviderProfile,
  TimerPort,
} from "./types.ts";

const defaultTimer: TimerPort = {
  setTimeout(callback, milliseconds) {
    return globalThis.setTimeout(callback, milliseconds);
  },
  clearTimeout(handle) {
    globalThis.clearTimeout(handle as ReturnType<typeof globalThis.setTimeout>);
  },
};

export class OpenAICompatibleModelGateway {
  readonly profile: ProviderProfile;
  #apiKey: string;
  private readonly fetch: FetchLike;
  private readonly timer: TimerPort;
  private readonly defaultTimeoutMs: number;
  private readonly maxRequestBytes: number;

  constructor(config: ModelGatewayConfig) {
    if (!config.apiKey.trim()) {
      throw new ProviderError({
        kind: "configuration",
        providerId: config.profile.id,
        message: `API key is required; resolve ${config.profile.apiKeyEnvironmentVariable} in the host`,
      });
    }
    const defaultTimeoutMs = config.defaultTimeoutMs ?? 60_000;
    const maxRequestBytes = config.maxRequestBytes ?? 16 * 1024 * 1024;
    if (!Number.isInteger(defaultTimeoutMs) || defaultTimeoutMs < 1) {
      throw new ProviderError({
        kind: "configuration",
        providerId: config.profile.id,
        message: "defaultTimeoutMs must be a positive integer",
      });
    }
    if (!Number.isInteger(maxRequestBytes) || maxRequestBytes < 1) {
      throw new ProviderError({
        kind: "configuration",
        providerId: config.profile.id,
        message: "maxRequestBytes must be a positive integer",
      });
    }
    this.profile = config.profile;
    this.#apiKey = config.apiKey;
    this.fetch = config.fetch ?? defaultFetch;
    this.timer = config.timer ?? defaultTimer;
    this.defaultTimeoutMs = defaultTimeoutMs;
    this.maxRequestBytes = maxRequestBytes;
  }

  async complete(
    request: ModelRequest,
    options: ModelGatewayOptions = {},
  ): Promise<ModelCompletion> {
    const built = buildChatRequest(this.profile, request, false);
    const abort = this.createAbortScope(options);
    try {
      const response = await this.send(built.url, built.body, abort.signal);
      const payload = await response.json().catch(() => {
        throw new ResponseProtocolError("Completion response is not valid JSON");
      });
      return parseCompletionResponse(this.profile, payload);
    } catch (error) {
      throw this.normalizeFailure(error, abort);
    } finally {
      abort.dispose();
    }
  }

  async *stream(
    request: ModelRequest,
    options: ModelGatewayOptions = {},
  ): AsyncGenerator<ModelStreamEvent> {
    const built = buildChatRequest(this.profile, request, true);
    const abort = this.createAbortScope(options);
    let started = false;
    let streamId: string | undefined;
    let streamModel: string | undefined;
    let cumulativeContent = "";
    let cumulativeReasoning = "";
    let finishReason: ModelStreamEvent & { readonly type: "finish" } = {
      type: "finish",
      reason: "unknown",
    };
    try {
      const response = await this.send(built.url, built.body, abort.signal);
      if (!response.body) {
        throw new ResponseProtocolError("Streaming response has no body");
      }
      for await (const event of parseServerSentEvents(response.body)) {
        let payload: unknown;
        try {
          payload = JSON.parse(event);
        } catch {
          throw new ResponseProtocolError("SSE data is not valid JSON");
        }
        const chunk = parseStreamChunk(payload);
        streamId ??= chunk.id;
        streamModel ??= chunk.model;

        const hasOutput = Boolean(
          chunk.content
          || chunk.reasoningContent
          || chunk.reasoningDetailsText
          || chunk.toolCallDeltas.length
          || chunk.finishReason,
        );
        if (!started && hasOutput) {
          if (!streamId || !streamModel) {
            throw new ResponseProtocolError("First output chunk must include id and model");
          }
          started = true;
          yield {
            type: "start",
            id: streamId,
            providerId: this.profile.id,
            model: streamModel,
          };
        }

        if (chunk.reasoningDetailsText !== undefined) {
          const normalized = normalizePiece(
            "cumulative",
            cumulativeReasoning,
            chunk.reasoningDetailsText,
          );
          cumulativeReasoning = normalized.next;
          if (normalized.delta) yield { type: "reasoning_delta", text: normalized.delta };
        } else if (chunk.reasoningContent !== undefined) {
          const normalized = normalizePiece(
            this.profile.streamContentMode,
            cumulativeReasoning,
            chunk.reasoningContent,
          );
          cumulativeReasoning = normalized.next;
          if (normalized.delta) yield { type: "reasoning_delta", text: normalized.delta };
        }
        if (chunk.content !== undefined) {
          const normalized = normalizePiece(
            this.profile.streamContentMode,
            cumulativeContent,
            chunk.content,
          );
          cumulativeContent = normalized.next;
          if (normalized.delta) yield { type: "text_delta", text: normalized.delta };
        }
        for (const call of chunk.toolCallDeltas) {
          yield { type: "tool_call_delta", ...call };
        }
        if (chunk.usage) yield { type: "usage", usage: chunk.usage };
        if (chunk.finishReason) finishReason = { type: "finish", reason: chunk.finishReason };
      }
      if (!started) {
        throw new ResponseProtocolError("Stream ended before an output chunk was received");
      }
      yield finishReason;
    } catch (error) {
      throw this.normalizeFailure(error, abort);
    } finally {
      abort.dispose();
    }
  }

  private async send(
    url: string,
    body: Readonly<Record<string, unknown>>,
    signal: AbortSignal,
  ): Promise<HttpResponseLike> {
    const serializedBody = JSON.stringify(body);
    if (new TextEncoder().encode(serializedBody).byteLength > this.maxRequestBytes) {
      throw new ProviderError({
        kind: "invalid_request",
        providerId: this.profile.id,
        message: `Request body exceeds the configured ${this.maxRequestBytes}-byte limit`,
      });
    }
    let response: HttpResponseLike;
    try {
      response = await this.fetch(url, {
        method: "POST",
        headers: {
          Authorization: `Bearer ${this.#apiKey}`,
          "Content-Type": "application/json",
          Accept: "application/json, text/event-stream",
        },
        body: serializedBody,
        signal,
      });
    } catch (error) {
      throw error;
    }
    if (!response.ok) throw await this.httpError(response);
    return response;
  }

  private async httpError(response: HttpResponseLike): Promise<ProviderError> {
    let payload: unknown;
    try {
      payload = await response.json();
    } catch {
      payload = undefined;
    }
    const extracted = extractRemoteError(payload);
    const kind = errorKindForStatus(response.status, extracted.code);
    const requestId = firstHeader(response, [
      "x-request-id",
      "request-id",
      "x-dashscope-request-id",
    ]);
    return new ProviderError({
      kind,
      providerId: this.profile.id,
      message: redactSecret(
        extracted.message || `${this.profile.label} request failed with HTTP ${response.status}`,
        this.#apiKey,
      ),
      status: response.status,
      code: extracted.code,
      requestId,
      retryAfterMs: parseRetryAfter(response.headers.get("retry-after")),
      retriable: response.status === 408 || response.status === 429 || response.status >= 500,
    });
  }

  private normalizeFailure(
    error: unknown,
    abort: AbortScope,
  ): ProviderError {
    if (error instanceof ProviderError) return error;
    if (abort.timedOut()) {
      return new ProviderError({
        kind: "timeout",
        providerId: this.profile.id,
        message: `${this.profile.label} request timed out`,
        retriable: true,
      });
    }
    if (abort.externalAborted() || isAbortError(error)) {
      return new ProviderError({
        kind: "cancelled",
        providerId: this.profile.id,
        message: `${this.profile.label} request was cancelled`,
      });
    }
    if (error instanceof ResponseProtocolError || error instanceof SseProtocolError) {
      return new ProviderError({
        kind: "protocol",
        providerId: this.profile.id,
        message: error.message,
      });
    }
    return new ProviderError({
      kind: "network",
      providerId: this.profile.id,
      message: `${this.profile.label} network request failed`,
      retriable: true,
    });
  }

  private createAbortScope(options: ModelGatewayOptions): AbortScope {
    const timeoutMs = options.timeoutMs ?? this.defaultTimeoutMs;
    if (!Number.isInteger(timeoutMs) || timeoutMs < 1) {
      throw new ProviderError({
        kind: "configuration",
        providerId: this.profile.id,
        message: "timeoutMs must be a positive integer",
      });
    }
    return createAbortScope(options.signal, timeoutMs, this.timer);
  }
}

interface AbortScope {
  readonly signal: AbortSignal;
  timedOut(): boolean;
  externalAborted(): boolean;
  dispose(): void;
}

function createAbortScope(
  external: AbortSignal | undefined,
  timeoutMs: number,
  timer: TimerPort,
): AbortScope {
  const controller = new AbortController();
  let didTimeout = false;
  let didExternalAbort = external?.aborted ?? false;
  const onExternalAbort = () => {
    didExternalAbort = true;
    controller.abort(external?.reason);
  };
  if (external?.aborted) controller.abort(external.reason);
  else external?.addEventListener("abort", onExternalAbort, { once: true });
  const handle = timer.setTimeout(() => {
    didTimeout = true;
    controller.abort(new DOMException("Request timed out", "TimeoutError"));
  }, timeoutMs);
  return {
    signal: controller.signal,
    timedOut: () => didTimeout,
    externalAborted: () => didExternalAbort,
    dispose() {
      timer.clearTimeout(handle);
      external?.removeEventListener("abort", onExternalAbort);
    },
  };
}

function normalizePiece(
  mode: "delta" | "cumulative",
  previous: string,
  current: string,
): { readonly delta: string; readonly next: string } {
  if (mode === "delta") return { delta: current, next: previous + current };
  if (current.startsWith(previous)) {
    return { delta: current.slice(previous.length), next: current };
  }
  if (previous.endsWith(current)) return { delta: "", next: previous };
  return { delta: current, next: previous + current };
}

function errorKindForStatus(status: number, code?: string): ProviderErrorKind {
  const normalizedCode = code?.toLowerCase() ?? "";
  if (
    normalizedCode.includes("content_filter")
    || normalizedCode.includes("sensitive")
    || normalizedCode.includes("safety")
  ) {
    return "content_filter";
  }
  if (status === 401) return "authentication";
  if (status === 402 || normalizedCode.includes("quota") || normalizedCode.includes("balance")) {
    return "quota";
  }
  if (status === 403) return "permission";
  if (status === 429) return "rate_limit";
  if (status === 400 || status === 404 || status === 409 || status === 422) {
    return "invalid_request";
  }
  if (status >= 500 || status === 408) return "server";
  return "server";
}

function extractRemoteError(input: unknown): {
  readonly message?: string;
  readonly code?: string;
} {
  if (!input || typeof input !== "object" || Array.isArray(input)) return {};
  const root = input as Record<string, unknown>;
  const nested = root.error && typeof root.error === "object" && !Array.isArray(root.error)
    ? root.error as Record<string, unknown>
    : root;
  return {
    message: typeof nested.message === "string" ? nested.message.slice(0, 1000) : undefined,
    code: typeof nested.code === "string"
      ? nested.code
      : typeof nested.type === "string"
        ? nested.type
        : undefined,
  };
}

function firstHeader(response: HttpResponseLike, names: readonly string[]): string | undefined {
  for (const name of names) {
    const value = response.headers.get(name);
    if (value) return value;
  }
  return undefined;
}

function parseRetryAfter(value: string | null): number | undefined {
  if (!value) return undefined;
  const seconds = Number(value);
  if (Number.isFinite(seconds) && seconds >= 0) return Math.ceil(seconds * 1000);
  const date = Date.parse(value);
  return Number.isFinite(date) ? Math.max(0, date - Date.now()) : undefined;
}

function redactSecret(message: string, secret: string): string {
  return message.includes(secret) ? message.split(secret).join("[REDACTED]") : message;
}

function isAbortError(error: unknown): boolean {
  return error instanceof DOMException && error.name === "AbortError";
}

const defaultFetch: FetchLike = async (input, init) => {
  const response = await globalThis.fetch(input, init);
  return response as unknown as HttpResponseLike;
};
