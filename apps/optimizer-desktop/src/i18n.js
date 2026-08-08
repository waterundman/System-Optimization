// v0.8.0 Stage 2: Lightweight i18n infrastructure.
// No runtime third-party dependencies — pure ES Module dictionary + t(key, params).
// Fallback chain: current locale -> zh-CN -> en-US -> key.

import zhCN from "./locales/zh-CN.js";
import enUS from "./locales/en-US.js";

const dictionaries = new Map();
let currentLocale = "zh-CN";

registerLocale("zh-CN", zhCN);
registerLocale("en-US", enUS);

export function registerLocale(locale, dictionary) {
  if (typeof locale !== "string" || locale.length === 0) return;
  if (!dictionary || typeof dictionary !== "object") return;
  dictionaries.set(locale, dictionary);
}

export function setLocale(locale) {
  if (typeof locale === "string" && locale.length > 0) {
    currentLocale = locale;
  }
}

export function getLocale() {
  return currentLocale;
}

export function t(key, params) {
  const chain = [currentLocale, "zh-CN", "en-US"];
  for (const locale of chain) {
    const dict = dictionaries.get(locale);
    if (dict) {
      const value = resolveKey(dict, key);
      if (typeof value === "string") {
        return interpolate(value, params);
      }
    }
  }
  return key;
}

export function tf(key, params, fallback) {
  const translated = t(key, params);
  if (translated === key && typeof fallback === "string") {
    return interpolate(fallback, params);
  }
  return translated;
}

function resolveKey(dict, key) {
  if (Object.prototype.hasOwnProperty.call(dict, key)) {
    return dict[key];
  }
  const separator = ".";
  if (key.indexOf(separator) === -1) return undefined;
  const parts = key.split(separator);
  let current = dict;
  for (const part of parts) {
    if (current && typeof current === "object" && Object.prototype.hasOwnProperty.call(current, part)) {
      current = current[part];
    } else {
      return undefined;
    }
  }
  return typeof current === "string" ? current : undefined;
}

function interpolate(template, params) {
  if (!params || typeof params !== "object") return template;
  return template.replace(/\{(\w+)\}/g, (match, name) => {
    return Object.prototype.hasOwnProperty.call(params, name)
      ? String(params[name])
      : match;
  });
}
