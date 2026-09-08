// Deprecated: canonical JSON is now implemented by @optimizer/protocol's
// stableStringify. This module is kept as a one-line compatibility alias so
// the editor-bridge public surface (canonicalJson) stays intact; new code
// should import stableStringify from @optimizer/protocol directly.
// TODO(cleanup): remove this file once the compatibility alias is unused.
export { stableStringify as canonicalJson } from "@optimizer/protocol";
