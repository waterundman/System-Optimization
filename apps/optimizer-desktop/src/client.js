import { t } from "./i18n.js";

// v0.9.0 Stage 1: host IPC convergence. All Tauri `core.invoke` traffic is
// funnelled through this module so frontend domains (welcome / workspace /
// drawers / review) never touch `window.__TAURI__` directly. The original
// `invokeHost` signature is preserved as the compatible export for modules
// that already reference it.
//
// Error classification: host failures are tagged with a coarse `kind`
// (network / authorization / business / unknown) via the `__hostErrorKind`
// field on the thrown error object. `normalizeHostError` in frontend-state.js
// still reads only `code` / `message`, so downstream behaviour is unchanged.
export function classifyHostError(error) {
  const code = error && typeof error === "object" && typeof error.code === "string"
    ? error.code
    : "HOST_ERROR";
  if (code === "HOST_UNAVAILABLE") return "network";
  if (/^(?:AUTH|UNAUTHORIZED|FORBIDDEN)/u.test(code)) return "authorization";
  if (/CREDENTIAL|SECRET|KEY/u.test(code)) return "authorization";
  if (
    /^(?:INVALID|CONFLICT|NO_CHANGES|TARGET|REVIEW|STYLE|SAVE|IMPORT|EXPORT|PROVIDER|OPERATION|MODEL|CONTEXT|RECENT|CHECKPOINT|SUMMARY)/u
      .test(code)
  ) {
    return "business";
  }
  return "unknown";
}

async function invokeHost(command, args = {}) {
  const invoke = window.__TAURI__?.core?.invoke;
  if (typeof invoke !== "function") {
    throw { code: "HOST_UNAVAILABLE", message: t("error.hostUnavailable") };
  }
  try {
    return await invoke(command, args);
  } catch (error) {
    if (error && typeof error === "object" && typeof error.code === "string") {
      throw { ...error, __hostErrorKind: classifyHostError(error) };
    }
    throw error;
  }
}

export { invokeHost };
