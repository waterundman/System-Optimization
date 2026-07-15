import type {
  FinishReason,
  ModelCompletion,
  ModelToolCall,
  ModelUsage,
  ProviderProfile,
} from "./types.ts";

export interface ParsedStreamChunk {
  readonly id?: string;
  readonly model?: string;
  readonly content?: string;
  readonly reasoningContent?: string;
  readonly reasoningDetailsText?: string;
  readonly toolCallDeltas: readonly {
    readonly index: number;
    readonly id?: string;
    readonly name?: string;
    readonly arguments?: string;
  }[];
  readonly finishReason?: FinishReason;
  readonly usage?: ModelUsage;
}

export class ResponseProtocolError extends Error {
  constructor(message: string) {
    super(message);
    this.name = "ResponseProtocolError";
  }
}

export function parseCompletionResponse(
  profile: ProviderProfile,
  input: unknown,
): ModelCompletion {
  const root = requireRecord(input, "completion response");
  const choice = requireRecord(requireArray(root.choices, "choices")[0], "choices[0]");
  const message = requireRecord(choice.message, "choices[0].message");
  const content = nullableString(message.content, "message.content") ?? "";
  const reasoningContent = extractReasoning(message);
  return {
    id: requireString(root.id, "id"),
    providerId: profile.id,
    model: requireString(root.model, "model"),
    content,
    reasoningContent,
    toolCalls: parseToolCalls(message.tool_calls),
    finishReason: normalizeFinishReason(choice.finish_reason),
    usage: parseUsage(root.usage),
  };
}

export function parseStreamChunk(input: unknown): ParsedStreamChunk {
  const root = requireRecord(input, "stream chunk");
  const choices = root.choices === undefined ? [] : requireArray(root.choices, "choices");
  const first = choices[0];
  let content: string | undefined;
  let reasoningContent: string | undefined;
  let reasoningDetailsText: string | undefined;
  let toolCallDeltas: ParsedStreamChunk["toolCallDeltas"] = [];
  let finishReason: FinishReason | undefined;
  if (first !== undefined) {
    const choice = requireRecord(first, "choices[0]");
    const delta = requireRecord(choice.delta, "choices[0].delta");
    content = optionalString(delta.content, "delta.content");
    reasoningContent = optionalString(delta.reasoning_content, "delta.reasoning_content")
      ?? optionalString(delta.reasoning, "delta.reasoning");
    reasoningDetailsText = extractReasoningDetails(delta.reasoning_details);
    toolCallDeltas = parseToolCallDeltas(delta.tool_calls);
    if (choice.finish_reason !== null && choice.finish_reason !== undefined) {
      finishReason = normalizeFinishReason(choice.finish_reason);
    }
  }
  return {
    id: optionalString(root.id, "id"),
    model: optionalString(root.model, "model"),
    content,
    reasoningContent,
    reasoningDetailsText,
    toolCallDeltas,
    finishReason,
    usage: parseUsage(root.usage),
  };
}

export function parseUsage(input: unknown): ModelUsage | undefined {
  if (input === null || input === undefined) return undefined;
  const usage = requireRecord(input, "usage");
  const inputTokens = optionalInteger(usage.prompt_tokens)
    ?? optionalInteger(usage.input_tokens)
    ?? 0;
  const outputTokens = optionalInteger(usage.completion_tokens)
    ?? optionalInteger(usage.output_tokens)
    ?? 0;
  const totalTokens = optionalInteger(usage.total_tokens) ?? inputTokens + outputTokens;
  const promptDetails = optionalRecord(usage.prompt_tokens_details);
  const completionDetails = optionalRecord(usage.completion_tokens_details);
  const cachedInputTokens = optionalInteger(usage.cached_tokens)
    ?? optionalInteger(promptDetails?.cached_tokens)
    ?? optionalInteger(usage.prompt_cache_hit_tokens);
  const reasoningTokens = optionalInteger(completionDetails?.reasoning_tokens);
  return {
    inputTokens,
    outputTokens,
    totalTokens,
    ...(cachedInputTokens !== undefined ? { cachedInputTokens } : {}),
    ...(reasoningTokens !== undefined ? { reasoningTokens } : {}),
  };
}

function extractReasoning(message: Readonly<Record<string, unknown>>): string | undefined {
  return optionalString(message.reasoning_content, "message.reasoning_content")
    ?? optionalString(message.reasoning, "message.reasoning")
    ?? extractReasoningDetails(message.reasoning_details);
}

function extractReasoningDetails(input: unknown): string | undefined {
  if (input === undefined || input === null) return undefined;
  const details = requireArray(input, "reasoning_details");
  const texts = details
    .map((detail, index) => optionalString(
      requireRecord(detail, `reasoning_details[${index}]`).text,
      `reasoning_details[${index}].text`,
    ))
    .filter((text): text is string => text !== undefined);
  return texts.length > 0 ? texts.join("") : undefined;
}

function parseToolCalls(input: unknown): readonly ModelToolCall[] {
  if (input === undefined || input === null) return [];
  return requireArray(input, "tool_calls").map((value, index) => {
    const call = requireRecord(value, `tool_calls[${index}]`);
    const fn = requireRecord(call.function, `tool_calls[${index}].function`);
    return {
      id: requireString(call.id, `tool_calls[${index}].id`),
      type: "function" as const,
      function: {
        name: requireString(fn.name, `tool_calls[${index}].function.name`),
        arguments: requireString(fn.arguments, `tool_calls[${index}].function.arguments`),
      },
    };
  });
}

function parseToolCallDeltas(input: unknown): ParsedStreamChunk["toolCallDeltas"] {
  if (input === undefined || input === null) return [];
  return requireArray(input, "delta.tool_calls").map((value, position) => {
    const call = requireRecord(value, `delta.tool_calls[${position}]`);
    const fn = optionalRecord(call.function);
    return {
      index: optionalInteger(call.index) ?? position,
      id: optionalString(call.id, `delta.tool_calls[${position}].id`),
      name: optionalString(fn?.name, `delta.tool_calls[${position}].function.name`),
      arguments: optionalString(
        fn?.arguments,
        `delta.tool_calls[${position}].function.arguments`,
      ),
    };
  });
}

function normalizeFinishReason(input: unknown): FinishReason {
  if (typeof input !== "string") return "unknown";
  if (
    input === "stop"
    || input === "length"
    || input === "content_filter"
    || input === "tool_calls"
    || input === "insufficient_system_resource"
  ) {
    return input;
  }
  return "unknown";
}

function requireRecord(input: unknown, name: string): Readonly<Record<string, unknown>> {
  if (!input || typeof input !== "object" || Array.isArray(input)) {
    throw new ResponseProtocolError(`${name} must be an object`);
  }
  return input as Readonly<Record<string, unknown>>;
}

function optionalRecord(input: unknown): Readonly<Record<string, unknown>> | undefined {
  if (input === undefined || input === null) return undefined;
  return requireRecord(input, "value");
}

function requireArray(input: unknown, name: string): readonly unknown[] {
  if (!Array.isArray(input)) throw new ResponseProtocolError(`${name} must be an array`);
  return input;
}

function requireString(input: unknown, name: string): string {
  if (typeof input !== "string" || !input) {
    throw new ResponseProtocolError(`${name} must be a non-empty string`);
  }
  return input;
}

function nullableString(input: unknown, name: string): string | undefined {
  if (input === null || input === undefined) return undefined;
  if (typeof input !== "string") throw new ResponseProtocolError(`${name} must be a string`);
  return input;
}

function optionalString(input: unknown, name: string): string | undefined {
  return nullableString(input, name);
}

function optionalInteger(input: unknown): number | undefined {
  return Number.isInteger(input) && Number(input) >= 0 ? Number(input) : undefined;
}
