import assert from "node:assert/strict";
import test from "node:test";
import {
  OpenAICompatibleModelGateway,
  ModelProviderRouter,
  ProviderError,
  buildChatRequest,
  createOpenAICompatibleProfile,
  createQwenProfile,
  deepSeekProfile,
  kimiProfile,
  miniMaxProfile,
  ollamaProfile,
  type FetchLike,
  type HttpRequestInit,
  type HttpResponseLike,
  type ModelRequest,
  type TimerPort,
} from "../src/index.ts";

const basicRequest: ModelRequest = {
  messages: [{ role: "user", content: "优化这段文字" }],
  maxOutputTokens: 512,
};

test("ships current official profiles without legacy DeepSeek model defaults", () => {
  assert.equal(deepSeekProfile.baseUrl, "https://api.deepseek.com");
  assert.equal(deepSeekProfile.defaultModel, "deepseek-v4-flash");
  assert.equal(deepSeekProfile.knownModels.includes("deepseek-chat"), false);
  assert.equal(kimiProfile.baseUrl, "https://api.moonshot.cn/v1");
  assert.equal(kimiProfile.defaultModel, "kimi-k2.6");
  assert.equal(miniMaxProfile.baseUrl, "https://api.minimaxi.com/v1");
  assert.equal(miniMaxProfile.defaultModel, "MiniMax-M3");
  assert.equal(Object.isFrozen(deepSeekProfile), true);
  assert.equal(Object.isFrozen(deepSeekProfile.knownModels), true);
  assert.equal(Object.isFrozen(deepSeekProfile.capabilities), true);
});

test("builds region-aware Qwen endpoints and validates workspace IDs", () => {
  assert.equal(
    createQwenProfile({ region: "singapore", workspaceId: "ws_123" }).baseUrl,
    "https://ws_123.ap-southeast-1.maas.aliyuncs.com/compatible-mode/v1",
  );
  assert.equal(
    createQwenProfile({ region: "us" }).baseUrl,
    "https://dashscope-us.aliyuncs.com/compatible-mode/v1",
  );
  assert.throws(
    () => createQwenProfile({ region: "japan" }),
    (error: unknown) => error instanceof ProviderError && error.kind === "configuration",
  );
  assert.throws(
    () => createQwenProfile({ workspaceId: "evil.example/path" }),
    (error: unknown) => error instanceof ProviderError && error.kind === "configuration",
  );
  assert.throws(
    () => createQwenProfile({ region: "unknown" as never }),
    (error: unknown) => error instanceof ProviderError && error.kind === "configuration",
  );
});

test("pins Ollama to the local OpenAI-compatible endpoint without credentials", async () => {
  const built = buildChatRequest(ollamaProfile, {
    ...basicRequest,
    reasoning: { mode: "disabled" },
    responseFormat: "json_object",
  }, true);
  assert.equal(ollamaProfile.locality, "local");
  assert.equal(ollamaProfile.authentication, "none");
  assert.equal(built.url, "http://127.0.0.1:11434/v1/chat/completions");
  assert.equal(built.body.max_tokens, 512);
  assert.equal(built.body.reasoning_effort, "none");

  assert.throws(
    () => buildChatRequest({ ...ollamaProfile, baseUrl: "http://example.com/v1" }, basicRequest, false),
    (error: unknown) => error instanceof ProviderError && error.kind === "configuration",
  );

  const calls: HttpRequestInit[] = [];
  const gateway = new OpenAICompatibleModelGateway({
    profile: ollamaProfile,
    fetch: async (_url, init) => {
      calls.push(init);
      return jsonResponse({
        id: "chat-local-1",
        model: "qwen3:8b",
        choices: [{ message: { content: "本地结果", reasoning: "本地推理" }, finish_reason: "stop" }],
      });
    },
  });
  const completion = await gateway.complete(basicRequest);
  assert.equal(completion.content, "本地结果");
  assert.equal(completion.reasoningContent, "本地推理");
  assert.equal(calls[0]?.headers.Authorization, undefined);
});

test("builds a conservative profile only from a Host-issued trusted endpoint", () => {
  const profile = createOpenAICompatibleProfile({
    id: `endpoint-${"a".repeat(32)}`,
    label: "Acme Gateway",
    baseUrl: "https://api.acme.ai/openai/v1",
    updatedAt: "2026-07-21T00:00:00Z",
    capabilities: {
      jsonObject: false,
      streamUsage: false,
      maxOutputTokenField: "max_completion_tokens",
    },
  }, "acme-writer");
  const built = buildChatRequest(profile, {
    ...basicRequest,
    responseFormat: "text",
  }, true);
  assert.equal(profile.id, "openai_compatible");
  assert.equal(profile.locality, "remote");
  assert.equal(built.url, "https://api.acme.ai/openai/v1/chat/completions");
  assert.equal(built.body.max_completion_tokens, 512);
  assert.equal("stream_options" in built.body, false);
  assert.equal("response_format" in built.body, false);
  assert.equal("thinking" in built.body, false);
  assert.equal("reasoning_effort" in built.body, false);
  assert.throws(
    () => buildChatRequest(profile, {
      ...basicRequest,
      reasoning: { mode: "enabled", effort: "high" },
      responseFormat: "text",
    }, true),
    (error: unknown) => error instanceof ProviderError && error.kind === "invalid_request",
  );
  assert.throws(
    () => buildChatRequest(profile, { ...basicRequest, toolChoice: "none" }, true),
    (error: unknown) => error instanceof ProviderError && error.kind === "invalid_request",
  );
  assert.throws(
    () => createOpenAICompatibleProfile({
      ...{
        id: "attacker-selected",
        label: "Bad",
        baseUrl: "https://attacker.example.net/v1",
        updatedAt: "2026-07-21T00:00:00Z",
        capabilities: {
          jsonObject: false,
          streamUsage: false,
          maxOutputTokenField: "max_tokens" as const,
        },
      },
    }, "model"),
    (error: unknown) => error instanceof ProviderError && error.kind === "configuration",
  );
});

test("maps DeepSeek reasoning and output parameters to its official dialect", () => {
  const built = buildChatRequest(deepSeekProfile, {
    ...basicRequest,
    reasoning: { mode: "enabled", effort: "max" },
    responseFormat: "json_object",
  }, true);
  assert.equal(built.url, "https://api.deepseek.com/chat/completions");
  assert.deepEqual(built.body.thinking, { type: "enabled" });
  assert.equal(built.body.reasoning_effort, "max");
  assert.equal(built.body.max_tokens, 512);
  assert.deepEqual(built.body.stream_options, { include_usage: true });
  assert.deepEqual(built.body.response_format, { type: "json_object" });
});

test("maps Qwen and Kimi thinking preservation without leaking vendor fields upward", () => {
  const qwen = buildChatRequest(createQwenProfile(), {
    ...basicRequest,
    reasoning: { mode: "disabled", preserve: true },
  }, false);
  assert.equal(qwen.body.enable_thinking, false);
  assert.equal(qwen.body.preserve_thinking, true);
  assert.equal(qwen.body.max_completion_tokens, 512);

  const kimi = buildChatRequest(kimiProfile, {
    ...basicRequest,
    reasoning: { mode: "enabled", preserve: true },
  }, false);
  assert.deepEqual(kimi.body.thinking, { type: "enabled", keep: "all" });
  assert.equal(kimi.body.max_tokens, 512);

  assert.throws(
    () => buildChatRequest(kimiProfile, {
      ...basicRequest,
      model: "kimi-k2.7-code",
      reasoning: { mode: "disabled" },
    }, false),
    (error: unknown) => error instanceof ProviderError && error.kind === "invalid_request",
  );
});

test("normalizes MiniMax thinking output and rejects unverified JSON mode", () => {
  const built = buildChatRequest(miniMaxProfile, {
    ...basicRequest,
    reasoning: { mode: "disabled" },
  }, false);
  assert.equal(built.body.reasoning_split, true);
  assert.deepEqual(built.body.thinking, { type: "disabled" });
  assert.equal(built.body.max_completion_tokens, 512);
  assert.throws(
    () => buildChatRequest(miniMaxProfile, {
      ...basicRequest,
      responseFormat: "json_object",
    }, false),
    (error: unknown) => error instanceof ProviderError && error.kind === "invalid_request",
  );
});

test("sends a secret only in Authorization and parses a complete response", async () => {
  const calls: Array<{ readonly url: string; readonly init: HttpRequestInit }> = [];
  const fetch: FetchLike = async (url, init) => {
    calls.push({ url, init });
    return jsonResponse({
      id: "chat-1",
      model: "deepseek-v4-flash",
      choices: [{
        message: {
          role: "assistant",
          content: "修改后的文本",
          reasoning_content: "简要推理",
          tool_calls: [{
            id: "call-1",
            type: "function",
            function: { name: "lookup", arguments: "{\"id\":1}" },
          }],
        },
        finish_reason: "tool_calls",
      }],
      usage: {
        prompt_tokens: 10,
        completion_tokens: 8,
        total_tokens: 18,
        prompt_cache_hit_tokens: 4,
        completion_tokens_details: { reasoning_tokens: 3 },
      },
    });
  };
  const gateway = new OpenAICompatibleModelGateway({
    profile: deepSeekProfile,
    apiKey: "secret-key",
    fetch,
  });
  const completion = await gateway.complete(basicRequest);

  assert.equal(calls.length, 1);
  assert.equal(Object.keys(gateway).includes("apiKey"), false);
  assert.equal(calls[0]?.init.headers.Authorization, "Bearer secret-key");
  assert.equal(calls[0]?.init.body.includes("secret-key"), false);
  assert.equal(completion.content, "修改后的文本");
  assert.equal(completion.reasoningContent, "简要推理");
  assert.equal(completion.finishReason, "tool_calls");
  assert.equal(completion.toolCalls[0]?.function.name, "lookup");
  assert.deepEqual(completion.usage, {
    inputTokens: 10,
    outputTokens: 8,
    totalTokens: 18,
    cachedInputTokens: 4,
    reasoningTokens: 3,
  });
});

test("parses MiniMax reasoning_details from a non-stream response", async () => {
  const gateway = new OpenAICompatibleModelGateway({
    profile: miniMaxProfile,
    apiKey: "minimax-key",
    fetch: async () => jsonResponse({
      id: "mm-1",
      model: "MiniMax-M3",
      choices: [{
        message: {
          role: "assistant",
          content: "最终文本",
          reasoning_details: [{ text: "推理" }, { text: "过程" }],
        },
        finish_reason: "stop",
      }],
    }),
  });
  const completion = await gateway.complete(basicRequest);
  assert.equal(completion.reasoningContent, "推理过程");
  assert.equal(completion.content, "最终文本");
});

test("normalizes rate limits, retry metadata and secret redaction", async () => {
  const gateway = new OpenAICompatibleModelGateway({
    profile: kimiProfile,
    apiKey: "credential-token",
    fetch: async () => jsonResponse(
      { error: { code: "rate_limit-credential-token", message: "token credential-token exceeded" } },
      429,
      { "retry-after": "2", "x-request-id": "request-credential-token" },
    ),
  });
  await assert.rejects(
    gateway.complete(basicRequest),
    (error: unknown) => {
      assert.equal(error instanceof ProviderError, true);
      if (!(error instanceof ProviderError)) return false;
      assert.equal(error.kind, "rate_limit");
      assert.equal(error.retryAfterMs, 2000);
      assert.equal(error.code, "rate_limit-[REDACTED]");
      assert.equal(error.requestId, "request-[REDACTED]");
      assert.equal(error.retriable, true);
      assert.equal(error.code?.includes("credential-token"), false);
      assert.equal(error.requestId?.includes("credential-token"), false);
      assert.equal(error.message.includes("credential-token"), false);
      return true;
    },
  );
});

test("redacts secrets before truncating provider error metadata", async () => {
  const secret = "credential-token";
  const gateway = new OpenAICompatibleModelGateway({
    profile: kimiProfile,
    apiKey: secret,
    fetch: async () => jsonResponse(
      {
        error: {
          code: `${"c".repeat(195)}${secret}`,
          message: `${"m".repeat(995)}${secret}`,
        },
      },
      429,
      { "x-request-id": `${"r".repeat(507)}${secret}` },
    ),
  });
  await assert.rejects(
    gateway.complete(basicRequest),
    (error: unknown) => {
      assert.equal(error instanceof ProviderError, true);
      if (!(error instanceof ProviderError)) return false;
      assert.equal(error.code?.length, 200);
      assert.equal(error.requestId?.length, 512);
      assert.equal(error.message.length, 1000);
      assert.equal(error.code?.includes("crede"), false);
      assert.equal(error.requestId?.includes("crede"), false);
      assert.equal(error.message.includes("crede"), false);
      return true;
    },
  );
});

test("streams split SSE frames and normalizes cumulative MiniMax fields", async () => {
  const frames = [
    'data: {"id":"stream-1","model":"MiniMax-M3","choices":[{"delta":{"reasoning_details":[{"text":"思"}],"content":"你"},"finish_reason":null}]}\n\n',
    'data: {"id":"stream-1","model":"MiniMax-M3","choices":[{"delta":{"reasoning_details":[{"text":"思考"}],"content":"你好"},"finish_reason":null}]}\n\n',
    'data: {"id":"stream-1","model":"MiniMax-M3","choices":[{"delta":{"content":"你好"},"finish_reason":"stop"}],"usage":{"prompt_tokens":3,"completion_tokens":2,"total_tokens":5}}\n\n',
    "data: [DONE]\n\n",
  ].join("");
  const gateway = new OpenAICompatibleModelGateway({
    profile: miniMaxProfile,
    apiKey: "minimax-key",
    fetch: async () => sseResponse([
      frames.slice(0, 13),
      frames.slice(13, 91),
      frames.slice(91),
    ]),
  });

  const events = [];
  for await (const event of gateway.stream(basicRequest)) events.push(event);
  assert.deepEqual(events, [
    { type: "start", id: "stream-1", providerId: "minimax", model: "MiniMax-M3" },
    { type: "reasoning_delta", text: "思" },
    { type: "text_delta", text: "你" },
    { type: "reasoning_delta", text: "考" },
    { type: "text_delta", text: "好" },
    {
      type: "usage",
      usage: { inputTokens: 3, outputTokens: 2, totalTokens: 5 },
    },
    { type: "finish", reason: "stop" },
  ]);
});

test("streams standard DeepSeek deltas and usage-only chunks", async () => {
  const frames = [
    'data: {"id":"ds-stream","model":"deepseek-v4-flash","choices":[{"delta":{"reasoning_content":"思考"},"finish_reason":null}]}\n\n',
    'data: {"id":"ds-stream","model":"deepseek-v4-flash","choices":[{"delta":{"content":"答案"},"finish_reason":"stop"}]}\n\n',
    'data: {"id":"ds-stream","model":"deepseek-v4-flash","choices":[],"usage":{"prompt_tokens":4,"completion_tokens":2,"total_tokens":6}}\n\n',
    "data: [DONE]\n\n",
  ].join("");
  const gateway = new OpenAICompatibleModelGateway({
    profile: deepSeekProfile,
    apiKey: "deepseek-key",
    fetch: async () => sseResponse([frames]),
  });
  const events = [];
  for await (const event of gateway.stream(basicRequest)) events.push(event);
  assert.deepEqual(events, [
    { type: "start", id: "ds-stream", providerId: "deepseek", model: "deepseek-v4-flash" },
    { type: "reasoning_delta", text: "思考" },
    { type: "text_delta", text: "答案" },
    { type: "usage", usage: { inputTokens: 4, outputTokens: 2, totalTokens: 6 } },
    { type: "finish", reason: "stop" },
  ]);
});

test("rejects a stream that ends without any finish chunk", async () => {
  const gateway = new OpenAICompatibleModelGateway({
    profile: deepSeekProfile,
    apiKey: "deepseek-key",
    fetch: async () => sseResponse([
      'data: {"id":"no-finish","model":"deepseek-v4-flash","choices":[{"delta":{"content":"你好"},"finish_reason":null}]}\n\n',
      'data: {"id":"no-finish","model":"deepseek-v4-flash","choices":[{"delta":{}}]}\n\n',
      "data: [DONE]\n\n",
    ]),
  });
  await assert.rejects(
    async () => {
      const events = [];
      for await (const event of gateway.stream(basicRequest)) events.push(event);
    },
    (error: unknown) => error instanceof ProviderError && error.kind === "protocol",
  );
});

test("classifies timeout and external cancellation separately", async () => {
  const abortingFetch: FetchLike = async (_url, init) => {
    if (init.signal.aborted) throw new DOMException("aborted", "AbortError");
    return new Promise((_resolve, reject) => {
      init.signal.addEventListener(
        "abort",
        () => reject(new DOMException("aborted", "AbortError")),
        { once: true },
      );
    });
  };
  const immediateTimer: TimerPort = {
    setTimeout(callback) {
      callback();
      return 1;
    },
    clearTimeout() {},
  };
  const timed = new OpenAICompatibleModelGateway({
    profile: deepSeekProfile,
    apiKey: "key",
    fetch: abortingFetch,
    timer: immediateTimer,
  });
  await assert.rejects(
    timed.complete(basicRequest),
    (error: unknown) => error instanceof ProviderError && error.kind === "timeout",
  );

  const controller = new AbortController();
  controller.abort();
  const cancelled = new OpenAICompatibleModelGateway({
    profile: deepSeekProfile,
    apiKey: "key",
    fetch: abortingFetch,
  });
  await assert.rejects(
    cancelled.complete(basicRequest, { signal: controller.signal }),
    (error: unknown) => error instanceof ProviderError && error.kind === "cancelled",
  );
});

test("rejects malformed provider responses as protocol errors", async () => {
  const gateway = new OpenAICompatibleModelGateway({
    profile: deepSeekProfile,
    apiKey: "key",
    fetch: async () => jsonResponse({ choices: [] }),
  });
  await assert.rejects(
    gateway.complete(basicRequest),
    (error: unknown) => error instanceof ProviderError && error.kind === "protocol",
  );
});

test("rejects oversized requests before any network side effect", async () => {
  let called = false;
  const gateway = new OpenAICompatibleModelGateway({
    profile: deepSeekProfile,
    apiKey: "key",
    maxRequestBytes: 10,
    fetch: async () => {
      called = true;
      return jsonResponse({});
    },
  });
  await assert.rejects(
    gateway.complete(basicRequest),
    (error: unknown) => error instanceof ProviderError && error.kind === "invalid_request",
  );
  assert.equal(called, false);
});

test("routes calls by provider ID and rejects duplicate or missing registrations", async () => {
  const deepseek = new OpenAICompatibleModelGateway({
    profile: deepSeekProfile,
    apiKey: "deepseek-key",
    fetch: async () => jsonResponse({
      id: "deepseek-routed",
      model: "deepseek-v4-flash",
      choices: [{ message: { content: "DeepSeek" }, finish_reason: "stop" }],
    }),
  });
  const kimi = new OpenAICompatibleModelGateway({
    profile: kimiProfile,
    apiKey: "kimi-key",
    fetch: async () => jsonResponse({
      id: "kimi-routed",
      model: "kimi-k2.6",
      choices: [{ message: { content: "Kimi" }, finish_reason: "stop" }],
    }),
  });
  const router = new ModelProviderRouter([kimi, deepseek]);
  assert.deepEqual(router.listProfiles().map((profile) => profile.id), ["deepseek", "kimi"]);
  assert.equal((await router.complete("kimi", basicRequest)).content, "Kimi");
  assert.throws(
    () => router.register(kimi),
    (error: unknown) => error instanceof ProviderError && error.kind === "configuration",
  );
  assert.throws(
    () => router.provider("minimax"),
    (error: unknown) => error instanceof ProviderError && error.kind === "configuration",
  );
});

function jsonResponse(
  payload: unknown,
  status = 200,
  headers: Readonly<Record<string, string>> = {},
): HttpResponseLike {
  return new Response(JSON.stringify(payload), {
    status,
    headers: { "Content-Type": "application/json", ...headers },
  }) as unknown as HttpResponseLike;
}

function sseResponse(parts: readonly string[]): HttpResponseLike {
  const encoder = new TextEncoder();
  const body = new ReadableStream<Uint8Array>({
    start(controller) {
      for (const part of parts) controller.enqueue(encoder.encode(part));
      controller.close();
    },
  });
  return new Response(body, {
    status: 200,
    headers: { "Content-Type": "text/event-stream" },
  }) as unknown as HttpResponseLike;
}
