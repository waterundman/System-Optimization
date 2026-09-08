import { t } from "./i18n.js";

const SVG_NAMESPACE = "http:" + "//www.w3.org/2000/svg";

function element(tag, options = {}, children = [], doc = (typeof document !== "undefined" ? document : null)) {
  // v0.7.0 Stage 1 (D7): SVG namespace support. Tags prefixed with "svg:"
  // are created via createElementNS so the SVG overlay and its <path>
  // children sit in the SVG namespace and are addressable by SVG-aware
  // CSS and querySelector. The "svg:" prefix is stripped from the
  // qualified name passed to createElementNS so the resulting element's
  // tagName is the plain local name (e.g. "svg", "path", "g") rather than
  // "svg:svg" — this matches the behaviour of real browsers and keeps
  // querySelectorAll("path") working in linkedom-based unit tests. Plain
  // HTML tags still use createElement so the v0.6.0 baseline callers are
  // unaffected.
  const isSvg = tag.startsWith("svg:");
  const qualifiedName = isSvg ? tag.slice("svg:".length) : tag;
  const item = isSvg
    ? doc.createElementNS(SVG_NAMESPACE, qualifiedName)
    : doc.createElement(tag);
  if (options.className) item.className = options.className;
  if (options.text !== undefined) item.textContent = options.text;
  if (options.type) item.type = options.type;
  if (options.name) item.name = options.name;
  if (options.value !== undefined) item.value = options.value;
  if (options.placeholder) item.placeholder = options.placeholder;
  if (options.title) item.title = options.title;
  if (options.disabled) item.disabled = true;
  if (options.attrs) {
    for (const [name, value] of Object.entries(options.attrs)) item.setAttribute(name, String(value));
  }
  for (const child of children.flat()) if (child) item.append(child);
  return item;
}

function button(label, className, onClick, options = {}, doc = (typeof document !== "undefined" ? document : null)) {
  const item = element("button", { className, text: label, type: "button", ...options }, [], doc);
  if (onClick) item.addEventListener("click", onClick);
  return item;
}

// SVG icon paths — 16x16 viewBox, 1.4 stroke, currentColor
const ICON_PATHS = Object.freeze({
  chapter: "M4 2h5l3 3v9H4z M9 2v3h3 M6 8h4 M6 10h3",
  text: "M3 4.5h10 M3 7.5h10 M3 10.5h7",
  plus: "M8 3v10 M3 8h10",
  up: "M8 3.5v9 M4.5 7L8 3.5L11.5 7",
  down: "M8 3.5v9 M4.5 9L8 12.5L11.5 9",
  edit: "M3 13l1-3 6-6 2 2-6 6z M9 4l2 2",
  close: "M4 4l8 8 M12 4l-8 8",
  continue: "M3 8h8 M8 5l3 3-3 3",
  polish: "M8 3L9.5 6.5L13 8L9.5 9.5L8 13L6.5 9.5L3 8L6.5 6.5Z",
  compress: "M2 8h4 M6 5l2 3-2 3 M14 8h-4 M10 5l-2 3 2 3",
  expand: "M8 3v4 M8 9v4 M5 6L8 3L11 6 M5 10L8 13L11 10",
  critique: "M3 3h10v7H7l-2 2v-2H3z M5.5 6h5 M5.5 8h3",
  download: "M8 2v6 M5.5 5L8 7.5 10.5 5 M3 13h10",
  upload: "M8 9V3 M5.5 6L8 3.5 10.5 6 M3 13h10",
  flag: "M4 2v12 M4 3h7l-1.5 2.5L11 8H4",
  indent: "M3 4h10 M3 8h6 M3 12h10 M9 6l2 2-2 2",
  outdent: "M3 4h10 M3 8h6 M3 12h10 M9 6l-2 2 2 2",
  restore: "M4 8a4 4 0 1 0 1.5-3 M4 4.5v2h2",
  compare: "M2 4h5v8H2z M9 4h5v8H9z",
  chart: "M3 13h10 M5 13V9 M8 13V5.5 M11 13V7.5",
  layers: "M8 3l5 3-5 3-5-3z M8 9l5 3-5 3-5-3",
  history: "M8 4a4 4 0 1 0 0.01 0z M8 6v2l1.5 1",
  timeline: "M4 3v10 M4 4h6 M4 8h4 M4 12h6",
  book: "M3 4h4.5v9H3z M8.5 4H13v9H8.5 M8 4v9",
  settings: "M8 5.5a2.5 2.5 0 1 0 0 5 2.5 2.5 0 0 0 0-5z M8 2v1.5 M8 12.5V14 M2 8h1.5 M12.5 8H14",
  backup: "M3 5l2-2h6l2 2v8H3z M8 8v3 M6 10l2 2 2-2",
});

function svgIcon(name, size = 14) {
  const d = ICON_PATHS[name];
  if (!d) return null;
  return element("svg:svg", {
    attrs: {
      viewBox: "0 0 16 16",
      width: String(size),
      height: String(size),
      fill: "none",
      stroke: "currentColor",
      "stroke-width": "1.4",
      "stroke-linecap": "round",
      "stroke-linejoin": "round",
      "aria-hidden": "true",
    },
  }, [element("svg:path", { attrs: { d } })]);
}

function iconOnlyButton(iconName, className, onClick, options = {}, doc = (typeof document !== "undefined" ? document : null)) {
  const item = element("button", { className, type: "button", ...options }, [], doc);
  const icon = svgIcon(iconName, 12);
  if (icon) item.append(icon);
  if (onClick) item.addEventListener("click", onClick);
  return item;
}

function iconTextButton(iconName, label, className, onClick, options = {}, doc = (typeof document !== "undefined" ? document : null)) {
  const item = element("button", { className, type: "button", ...options }, [], doc);
  const icon = svgIcon(iconName, 13);
  if (icon) item.append(icon);
  item.append(label);
  if (onClick) item.addEventListener("click", onClick);
  return item;
}

function labeledInput(label, name, placeholder, required, value = "") {
  const input = element("input", { name, placeholder, value, attrs: { autocomplete: "off" } });
  input.required = required;
  return element("label", { className: "field" }, [element("span", { text: label }), input]);
}

// 目录选择字段 — 文本输入 + 「浏览…」按钮，点击后通过系统目录选择器选取路径。
function directoryField(label, name, placeholder) {
  const input = element("input", { name, placeholder, attrs: { autocomplete: "off" } });
  const browse = button(t("welcome.create.browse"), "small-button", (event) => {
    event.preventDefault();
    const picker = element("input", {
      type: "file",
      attrs: { "aria-label": label, webkitdirectory: "", directory: "" },
    });
    picker.hidden = true;
    picker.addEventListener("change", () => {
      const file = picker.files?.[0];
      picker.remove();
      if (file?.path) input.value = file.path;
    }, { once: true });
    (document.body || document).append(picker);
    picker.click();
  });
  return element("label", { className: "field" }, [
    element("span", { text: label }),
    element("div", { className: "field-row" }, [input, browse]),
  ]);
}

function formValue(form, name) {
  return String(new FormData(form).get(name) ?? "").trim();
}

function setFormBusy(form, busy) {
  for (const control of form.elements) control.disabled = busy;
}

// v0.8.0 Stage 3 (a11y): linkedom's HTMLSelectElement.value is a getter-only
// property, so assigning .value throws in unit tests. Real browsers implement
// the setter. This helper tries the native setter first and falls back to
// marking the matching <option> as selected so the initial state is correct
// in both environments.
function setSelectValue(selectEl, value) {
  try {
    selectEl.value = value;
    return;
  } catch {
    // linkedom getter-only — fall through to option marking.
  }
  const options = selectEl.options || [];
  for (const option of options) {
    if (option.value === value) {
      option.setAttribute("selected", "selected");
    } else {
      option.removeAttribute("selected");
    }
  }
}

function checkboxField(label, name, checked) {
  const input = element("input", { type: "checkbox", name });
  input.checked = checked;
  return element("label", { className: "checkbox-field" }, [input, element("span", { text: label })]);
}

function passwordField(label, name, placeholder) {
  const input = element("input", { type: "password", name, placeholder, attrs: { autocomplete: "new-password" } });
  return element("label", { className: "field" }, [element("span", { text: label }), input]);
}

function formatDate(value) {
  const date = new Date(value);
  return Number.isNaN(date.valueOf()) ? value : new Intl.DateTimeFormat("zh-CN", { dateStyle: "short", timeStyle: "short" }).format(date);
}

export {
  element,
  button,
  svgIcon,
  iconOnlyButton,
  iconTextButton,
  labeledInput,
  directoryField,
  formValue,
  setFormBusy,
  setSelectValue,
  checkboxField,
  passwordField,
  formatDate,
};
