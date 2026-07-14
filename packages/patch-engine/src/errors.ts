export type PatchEngineErrorCode =
  | "TARGET_MISMATCH"
  | "TARGET_RANGE_INVALID"
  | "INPUT_TOO_LARGE"
  | "NO_CHANGES"
  | "TOO_MANY_HUNKS"
  | "PROPOSAL_INVALID"
  | "PROPOSAL_BINDING_MISMATCH"
  | "HUNK_NOT_FOUND"
  | "REVIEW_FINALIZED"
  | "REVIEW_INCOMPLETE";

export class PatchEngineError extends Error {
  readonly code: PatchEngineErrorCode;
  readonly details?: Readonly<Record<string, unknown>>;

  constructor(
    code: PatchEngineErrorCode,
    message: string,
    details?: Readonly<Record<string, unknown>>,
  ) {
    super(message);
    this.name = "PatchEngineError";
    this.code = code;
    this.details = details;
  }
}
