export type DomainErrorCategory =
  | "user_action"
  | "policy"
  | "validation"
  | "provider_transient"
  | "provider_permanent"
  | "storage"
  | "conflict"
  | "internal";

export class DomainError extends Error {
  readonly code: string;
  readonly category: DomainErrorCategory;
  readonly retryable: boolean;
  readonly details?: Readonly<Record<string, unknown>>;

  constructor(input: {
    code: string;
    category: DomainErrorCategory;
    message: string;
    retryable?: boolean;
    details?: Readonly<Record<string, unknown>>;
  }) {
    super(input.message);
    this.name = "DomainError";
    this.code = input.code;
    this.category = input.category;
    this.retryable = input.retryable ?? false;
    this.details = input.details;
  }
}

