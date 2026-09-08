import { ProviderError } from "./errors.ts";
import type { QwenDeploymentRegion } from "@optimizer/protocol";
import type { ProviderProfile } from "./types.ts";

const verifiedAt = "2026-07-14";
const commonCapabilities = {
  streaming: true,
  reasoning: true,
  toolCalls: true,
  usageInStream: true,
} as const;

export const deepSeekProfile: ProviderProfile = freezeProfile({
  id: "deepseek",
  label: "DeepSeek",
  dialect: "deepseek",
  locality: "remote",
  baseUrl: "https://api.deepseek.com",
  authentication: "bearer",
  apiKeyEnvironmentVariable: "DEEPSEEK_API_KEY",
  defaultModel: "deepseek-v4-flash",
  knownModels: ["deepseek-v4-flash", "deepseek-v4-pro"],
  maxOutputTokenField: "max_tokens",
  streamContentMode: "delta",
  capabilities: { ...commonCapabilities, jsonObject: true },
  documentationUrl: "https://api-docs.deepseek.com/",
  verifiedAt,
});

export const kimiProfile: ProviderProfile = freezeProfile({
  id: "kimi",
  label: "Kimi",
  dialect: "kimi",
  locality: "remote",
  baseUrl: "https://api.moonshot.cn/v1",
  authentication: "bearer",
  apiKeyEnvironmentVariable: "MOONSHOT_API_KEY",
  defaultModel: "kimi-k2.6",
  knownModels: ["kimi-k2.6", "kimi-k2.7-code"],
  maxOutputTokenField: "max_tokens",
  streamContentMode: "delta",
  capabilities: { ...commonCapabilities, jsonObject: true },
  documentationUrl: "https://platform.kimi.com/docs/api/chat",
  verifiedAt,
});

export const miniMaxProfile: ProviderProfile = freezeProfile({
  id: "minimax",
  label: "MiniMax",
  dialect: "minimax",
  locality: "remote",
  baseUrl: "https://api.minimaxi.com/v1",
  authentication: "bearer",
  apiKeyEnvironmentVariable: "MINIMAX_API_KEY",
  defaultModel: "MiniMax-M3",
  knownModels: [
    "MiniMax-M3",
    "MiniMax-M2.7",
    "MiniMax-M2.7-highspeed",
    "MiniMax-M2.5",
    "MiniMax-M2.5-highspeed",
  ],
  maxOutputTokenField: "max_completion_tokens",
  streamContentMode: "cumulative",
  capabilities: { ...commonCapabilities, jsonObject: false },
  documentationUrl: "https://platform.minimaxi.com/docs/api-reference/text-openai-api",
  verifiedAt,
});

export type QwenRegion = QwenDeploymentRegion;

export function createQwenProfile(options: {
  readonly region?: QwenRegion;
  readonly workspaceId?: string;
} = {}): ProviderProfile {
  const region = options.region ?? "china";
  const workspaceId = options.workspaceId;
  if (!["china", "singapore", "us", "germany", "japan"].includes(region)) {
    throw configurationError(`Unsupported Qwen region: ${String(region)}`);
  }
  if (workspaceId !== undefined && !/^[A-Za-z0-9_-]+$/.test(workspaceId)) {
    throw configurationError("Qwen workspaceId contains unsupported characters");
  }
  const baseUrl = qwenBaseUrl(region, workspaceId);
  return freezeProfile({
    id: "qwen",
    label: "Qwen / Alibaba Cloud Model Studio",
    dialect: "qwen",
    locality: "remote",
    baseUrl,
    authentication: "bearer",
    apiKeyEnvironmentVariable: "DASHSCOPE_API_KEY",
    defaultModel: "qwen-plus",
    knownModels: ["qwen-plus", "qwen3.7-plus", "qwen3.7-flash"],
    maxOutputTokenField: "max_completion_tokens",
    streamContentMode: "delta",
    capabilities: { ...commonCapabilities, jsonObject: true },
    documentationUrl: "https://help.aliyun.com/en/model-studio/qwen-api-via-openai-chat-completions",
    verifiedAt,
  });
}

export const qwenProfile = createQwenProfile();

export const ollamaProfile: ProviderProfile = freezeProfile({
  id: "ollama",
  label: "Ollama (local)",
  dialect: "ollama",
  locality: "local",
  baseUrl: "http://127.0.0.1:11434/v1",
  authentication: "none",
  defaultModel: "qwen3:8b",
  knownModels: ["qwen3:8b", "llama3.2"],
  maxOutputTokenField: "max_tokens",
  streamContentMode: "delta",
  capabilities: { ...commonCapabilities, jsonObject: true },
  documentationUrl: "https://docs.ollama.com/api/openai-compatibility",
  verifiedAt: "2026-07-15",
});

export interface TrustedOpenAICompatibleEndpointProfileInput {
  readonly id: string;
  readonly label: string;
  readonly baseUrl: string;
  readonly updatedAt: string;
  readonly capabilities: {
    readonly jsonObject: boolean;
    readonly streamUsage: boolean;
    readonly maxOutputTokenField: "max_tokens" | "max_completion_tokens";
  };
}

export function createOpenAICompatibleProfile(
  endpoint: TrustedOpenAICompatibleEndpointProfileInput,
  defaultModel: string,
): ProviderProfile {
  if (!/^endpoint-[a-f0-9]{32}$/.test(endpoint.id)) {
    throw new ProviderError({
      kind: "configuration",
      providerId: "openai_compatible",
      message: "OpenAI-compatible endpoint ID is not Host-issued",
    });
  }
  return freezeProfile({
    id: "openai_compatible",
    label: endpoint.label,
    dialect: "openai_compatible",
    locality: "remote",
    baseUrl: endpoint.baseUrl,
    authentication: "bearer",
    defaultModel: defaultModel.trim(),
    knownModels: [],
    maxOutputTokenField: endpoint.capabilities.maxOutputTokenField,
    streamContentMode: "delta",
    capabilities: {
      streaming: true,
      reasoning: false,
      jsonObject: endpoint.capabilities.jsonObject,
      toolCalls: false,
      usageInStream: endpoint.capabilities.streamUsage,
    },
    documentationUrl: endpoint.baseUrl,
    verifiedAt: endpoint.updatedAt.slice(0, 10),
  });
}

export const officialProviderProfiles: Readonly<Record<Exclude<ProviderProfile["id"], "openai_compatible">, ProviderProfile>> = Object.freeze({
  deepseek: deepSeekProfile,
  qwen: qwenProfile,
  kimi: kimiProfile,
  minimax: miniMaxProfile,
  ollama: ollamaProfile,
});

function qwenBaseUrl(region: QwenRegion, workspaceId?: string): string {
  if (region === "us") return "https://dashscope-us.aliyuncs.com/compatible-mode/v1";
  if (!workspaceId) {
    if (region === "china") return "https://dashscope.aliyuncs.com/compatible-mode/v1";
    if (region === "singapore") return "https://dashscope-intl.aliyuncs.com/compatible-mode/v1";
    throw configurationError(`Qwen region ${region} requires a workspaceId`);
  }
  const hosts: Readonly<Record<Exclude<QwenRegion, "us">, string>> = {
    china: "cn-beijing.maas.aliyuncs.com",
    singapore: "ap-southeast-1.maas.aliyuncs.com",
    germany: "eu-central-1.maas.aliyuncs.com",
    japan: "ap-northeast-1.maas.aliyuncs.com",
  };
  return `https://${workspaceId}.${hosts[region]}/compatible-mode/v1`;
}

function configurationError(message: string): ProviderError {
  return new ProviderError({
    kind: "configuration",
    providerId: "qwen",
    message,
  });
}

function freezeProfile(profile: ProviderProfile): ProviderProfile {
  return Object.freeze({
    ...profile,
    knownModels: Object.freeze([...profile.knownModels]),
    capabilities: Object.freeze({ ...profile.capabilities }),
  });
}
