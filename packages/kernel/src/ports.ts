import type {
  Authority,
  CanonicalStatus,
  ContextTier,
  OperationIntent,
  RenderMode,
  Sensitivity,
} from "@optimizer/protocol";

export interface ContextCandidate {
  readonly id: string;
  readonly sourceRef: string;
  readonly sourceHash: string;
  readonly tier: ContextTier;
  readonly status: CanonicalStatus;
  readonly authority: Authority;
  readonly sensitivity: Sensitivity;
  readonly renderMode: RenderMode;
  readonly reasonCodes: readonly string[];
  readonly content: string;
  readonly mandatory?: boolean;
  readonly selectedByUser?: boolean;
  readonly signals?: {
    readonly relevance?: number;
    readonly structuralProximity?: number;
    readonly freshness?: number;
    readonly risk?: number;
  };
}

export interface ContextSource {
  readonly id: string;
  collect(intent: OperationIntent): Promise<readonly ContextCandidate[]>;
}

export interface TokenEstimator {
  readonly id: string;
  estimate(text: string): number;
}

export interface ContentHasher {
  readonly id: string;
  sha256(value: string): Promise<string>;
}

export interface Clock {
  now(): string;
}

export interface IdGenerator {
  next(prefix: "ctx" | "event" | "proposal"): string;
}

export interface TelemetryEvent {
  readonly name: string;
  readonly attributes: Readonly<Record<string, string | number | boolean>>;
}

export interface TelemetrySink {
  emit(event: TelemetryEvent): void;
}

export interface KernelPorts {
  readonly contextSources: readonly ContextSource[];
  readonly tokenizer: TokenEstimator;
  readonly hasher: ContentHasher;
  readonly clock: Clock;
  readonly ids: IdGenerator;
  readonly telemetry?: TelemetrySink;
}

