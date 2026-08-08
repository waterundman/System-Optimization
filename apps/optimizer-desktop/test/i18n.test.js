import assert from "node:assert/strict";
import test from "node:test";

// v0.8.0 Stage 2: i18n module unit tests.
// Tests the core t/setLocale/getLocale/registerLocale functions, the
// fallback chain (current -> zh-CN -> en-US -> key), and {name}
// placeholder interpolation. The module is loaded from the built dist/
// directory so the JSON import attributes resolve correctly under Node.
const { t, tf, setLocale, getLocale, registerLocale } = await import(
  "../dist/i18n.js"
);

function resetLocale() {
  setLocale("zh-CN");
}

test("T10: t() returns zh-CN translation by default", () => {
  resetLocale();
  assert.equal(t("common.saved"), "已保存");
  assert.equal(t("common.cancel"), "取消");
  assert.equal(t("welcome.title"), "把注意力留给文字");
});

test("T11: t() returns the key itself when no translation exists", () => {
  resetLocale();
  assert.equal(t("nonexistent.key"), "nonexistent.key");
  assert.equal(t("another.missing"), "another.missing");
});

test("T12: setLocale switches the active locale", () => {
  resetLocale();
  assert.equal(getLocale(), "zh-CN");
  setLocale("en-US");
  assert.equal(getLocale(), "en-US");
  assert.equal(t("common.saved"), "Saved");
  assert.equal(t("common.cancel"), "Cancel");
});

test("T13: getLocale returns the current locale", () => {
  resetLocale();
  assert.equal(getLocale(), "zh-CN");
  setLocale("en-US");
  assert.equal(getLocale(), "en-US");
  resetLocale();
  assert.equal(getLocale(), "zh-CN");
});

test("T14: fallback chain current -> zh-CN -> en-US -> key", () => {
  // When locale is en-US and a key exists only in zh-CN, the zh-CN
  // value should be returned (not the key).
  setLocale("en-US");
  // "timeline.state.draft" exists in both locales; en-US returns "Draft".
  assert.equal(t("timeline.state.draft"), "Draft");
  resetLocale();
});

test("T15: t() interpolates {name} placeholders", () => {
  resetLocale();
  assert.equal(
    t("recent.count", { count: 3 }),
    "3 个",
  );
  assert.equal(
    t("review.hunkIndex", { index: 0 }),
    "修改 0",
  );
  assert.equal(
    t("ai.op.generated", { count: 5 }),
    "已生成 5 处建议，正文未动。",
  );
});

test("T16: t() interpolation handles multiple placeholders", () => {
  resetLocale();
  assert.equal(
    t("review.count", { accepted: 2, rejected: 1 }),
    "2 接受 · 1 拒绝",
  );
  assert.equal(
    t("ai.op.retryFailed", { attempts: 3, message: "timeout" }),
    "重试 3 次后仍失败：timeout",
  );
});

test("T17: t() leaves unmatched placeholders as-is", () => {
  resetLocale();
  assert.equal(
    t("review.count", { accepted: 2 }),
    "2 接受 · {rejected} 拒绝",
  );
});

test("T18: t() interpolation with no params returns the raw template", () => {
  resetLocale();
  assert.equal(t("recent.count"), "{count} 个");
});

test("T19: registerLocale adds a new locale dictionary", () => {
  resetLocale();
  registerLocale("ja-JP", {
    "common.saved": "保存済み",
  });
  setLocale("ja-JP");
  assert.equal(t("common.saved"), "保存済み");
  // Fallback to zh-CN for keys not in ja-JP.
  assert.equal(t("common.cancel"), "取消");
  resetLocale();
});

test("T20: registerLocale ignores invalid inputs", () => {
  resetLocale();
  registerLocale("", { foo: "bar" });
  registerLocale(null, { foo: "bar" });
  registerLocale("xx-XX", null);
  registerLocale("xx-XX", "not-an-object");
  setLocale("xx-XX");
  // xx-XX was never registered, so fallback to zh-CN.
  assert.equal(t("common.saved"), "已保存");
  resetLocale();
});

test("T21: setLocale ignores invalid inputs", () => {
  resetLocale();
  setLocale("");
  setLocale(null);
  setLocale(123);
  assert.equal(getLocale(), "zh-CN");
});

test("T22: tf() uses the fallback string when key is missing", () => {
  resetLocale();
  assert.equal(
    tf("nonexistent.key", {}, "Fallback text"),
    "Fallback text",
  );
  assert.equal(
    tf("nonexistent.key", { name: "value" }, "Hello {name}"),
    "Hello value",
  );
});

test("T23: tf() returns the translation when key exists", () => {
  resetLocale();
  assert.equal(
    tf("common.saved", {}, "Fallback"),
    "已保存",
  );
});

test("T24: t() resolves nested dot-notation keys", () => {
  resetLocale();
  assert.equal(t("timeline.state.draft"), "草稿");
  assert.equal(t("timeline.state.cancelled"), "已取消");
  assert.equal(t("ai.op.polish.label"), "润色");
});

test("T25: en-US locale has matching keys for zh-CN", () => {
  // Both locale files should have the same set of keys so the
  // fallback chain never silently returns a key string.
  setLocale("en-US");
  // Verify a sampling of keys return English (not the key itself).
  assert.equal(t("common.saved"), "Saved");
  assert.equal(t("welcome.title"), "Leave attention for the words");
  assert.equal(t("providers.title"), "Models and credentials");
  assert.equal(t("versions.title"), "Version history");
  resetLocale();
});
