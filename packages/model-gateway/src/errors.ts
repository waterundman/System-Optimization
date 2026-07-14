import type { ProviderId } from "./types.ts";

export type ProviderErrorKind =
  | "authentication"
  | "permission"
  | "rate_limit"
  | "quota"
  | "invalid_request"
  | "content_filter"
  | "server"
  | "network"
  | "timeout"
  | "cancelled"
  | "protocol"
  | "configuration";

export class ProviderError extends Error {
  readonly kind: ProviderErrorKind;
  readonly providerId: ProviderId;
  readonly status?: number;
  readonly code?: string;
  readonly requestId?: string;
  readonly retryAfterMs?: number;
  readonly retriable: boolean;

  constructor(input: {
    readonly kind: ProviderErrorKind;
    readonly providerId: ProviderId;
    readonly message: string;
    readonly status?: number;
    readonly code?: string;
    readonly requestId?: string;
    readonly retryAfterMs?: number;
    readonly retriable?: boolean;
  }) {
    super(input.message);
    this.name = "ProviderError";
    this.kind = input.kind;
    this.providerId = input.providerId;
    this.status = input.status;
    this.code = input.code;
    this.requestId = input.requestId;
    this.retryAfterMs = input.retryAfterMs;
    this.retriable = input.retriable ?? false;
  }
}
