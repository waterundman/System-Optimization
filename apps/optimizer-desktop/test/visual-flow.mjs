import assert from "node:assert/strict";
import { mkdir } from "node:fs/promises";
import { createRequire } from "node:module";
import { dirname, resolve } from "node:path";
import { fileURLToPath } from "node:url";

const require = createRequire(import.meta.url);
const playwrightPackage = process.env.PLAYWRIGHT_PACKAGE;
const executablePath = process.env.PLAYWRIGHT_EXECUTABLE;
if (!playwrightPackage || !executablePath) {
  throw new Error("PLAYWRIGHT_PACKAGE and PLAYWRIGHT_EXECUTABLE are required");
}
const { chromium } = require(playwrightPackage);
const testDirectory = dirname(fileURLToPath(import.meta.url));
const screenshotDirectory = process.env.VISUAL_SCREENSHOT_DIR
  ?? resolve(testDirectory, "../../../.optimizer-cache/visual");
await mkdir(screenshotDirectory, { recursive: true });

const browser = await chromium.launch({
  headless: true,
  executablePath,
  args: ["--no-proxy-server"],
});
const page = await browser.newPage({ viewport: { width: 1280, height: 800 } });
const errors = [];
page.on("pageerror", (error) => errors.push(error.message));
page.on("console", (message) => {
  if (message.type() === "error") errors.push(message.text());
});
await page.addInitScript({ path: resolve(testDirectory, "browser-mock-host.js") });
await page.goto("http://127.0.0.1:4173/?recentWelcome=1", { waitUntil: "networkidle" });
await page.getByRole("heading", { name: "最近项目" }).waitFor();
await page.getByText("W:\\写作\\雾港来信.optimizer").waitFor();
await page.getByRole("button", { name: "打开最近项目 雾港来信" }).click();
await page.getByRole("heading", { name: "雾港来信" }).waitFor();

await page.getByRole("button", { name: "模型设置" }).click();
await page.getByText("凭据已存入系统保险库").waitFor();
await page.screenshot({ path: resolve(screenshotDirectory, "provider-settings.png"), fullPage: true });
await page.getByRole("button", { name: "Ollama / 本地" }).click();
await page.getByText("固定本地端点 · 无需 API Key").waitFor();
await page.getByText(/127\.0\.0\.1:11434\/v1/).waitFor();
await page.getByRole("button", { name: "检测本地模型" }).click();
await page.getByText("已发现 2 个本地模型。").waitFor();
await page.getByRole("checkbox", { name: "启用此供应商" }).check();
await page.getByRole("button", { name: "保存模型设置" }).click();
await page.getByText(/请求只会发往固定回环端点/).waitFor();
await page.screenshot({ path: resolve(screenshotDirectory, "ollama-settings.png"), fullPage: true });
await page.getByRole("button", { name: "DeepSeek" }).click();
await page.getByRole("button", { name: "收起模型" }).click();

await page.getByRole("button", { name: "知识 / 风格" }).click();
await page.getByRole("heading", { name: "项目知识与风格" }).waitFor();
await page.getByText("主角视觉").waitFor();
await page.getByText("克制的短句").waitFor();
await page.getByLabel("知识类型").selectOption("constraint");
await page.getByLabel("约束强度").selectOption("hard");
await page.getByLabel("发送策略").selectOption("never_send");
await page.getByLabel("标题").fill("禁止剧透");
await page.getByLabel("内容").fill("本章不得揭示凶手身份");
await page.getByRole("button", { name: "添加为 canonical" }).click();
await page.getByText(/项目知识已设为 canonical/).waitFor();
await page.getByText("禁止剧透").waitFor();
await page.screenshot({ path: resolve(screenshotDirectory, "style-library.png"), fullPage: true });
await page.getByRole("button", { name: "收起知识" }).click();

page.once("dialog", (dialog) => dialog.accept("第二章"));
await page.getByRole("button", { name: "新建顶层章节" }).click();
await page.getByRole("heading", { name: "第二章" }).waitFor();
await page.getByText(/已创建章节/).waitFor();
await page.screenshot({ path: resolve(screenshotDirectory, "document-created.png"), fullPage: true });

await page.getByRole("button", { name: "章 第二章", exact: true }).locator("..").hover();
page.once("dialog", (dialog) => dialog.accept("第二章：雨夜"));
await page.getByRole("button", { name: "重命名 第二章" }).click();
await page.getByRole("heading", { name: "第二章：雨夜" }).waitFor();
await page.getByRole("button", { name: "章 第二章：雨夜", exact: true }).locator("..").hover();
await page.getByRole("button", { name: "上移 第二章：雨夜" }).click();
await page.getByText(/已上移“第二章：雨夜”/).waitFor();
await page.getByRole("button", { name: "章 第二章：雨夜", exact: true }).locator("..").hover();
page.once("dialog", (dialog) => dialog.accept());
await page.getByRole("button", { name: "归档 第二章：雨夜" }).click();
await page.getByText("已归档 · 1").waitFor();
await page.getByRole("button", { name: "恢复 第二章：雨夜" }).click();
await page.getByRole("heading", { name: "第二章：雨夜" }).waitFor();
await page.getByRole("button", { name: "章 第二章：雨夜", exact: true }).locator("..").hover();
await page.getByRole("button", { name: "下移 第二章：雨夜" }).click();
await page.getByText(/已下移“第二章：雨夜”/).waitFor();
await page.getByRole("button", { name: "缩进 第二章：雨夜" }).click();
await page.getByText(/缩进为上一章节的子章节/).waitFor();
assert.equal(
  await page.getByRole("button", { name: "章 第二章：雨夜", exact: true }).locator(".document-indent").textContent(),
  "· ",
);
await page.getByRole("button", { name: "移出 第二章：雨夜" }).click();
await page.getByText(/移出到上一层/).waitFor();
await page.screenshot({ path: resolve(screenshotDirectory, "document-lifecycle.png"), fullPage: true });

const fileChooserPromise = page.waitForEvent("filechooser");
await page.getByRole("button", { name: "导入 MD" }).click();
const fileChooser = await fileChooserPromise;
await fileChooser.setFiles({
  name: "导入章.md",
  mimeType: "text/markdown",
  buffer: Buffer.from("# 导入章\n\n潮水正在上涨。", "utf8"),
});
await page.getByRole("heading", { name: "导入章" }).waitFor();
await page.getByText(/已导入 导入章\.md/).waitFor();
await page.getByRole("button", { name: "导出 MD" }).click();
await page.getByText(/optimizer-export-visual\.md/).waitFor();
await page.getByRole("button", { name: "章 雾港来信", exact: true }).click();
await page.getByText("摘要已就绪", { exact: true }).waitFor({ timeout: 10_000 });

const activeEditor = page.locator('[data-block-id="block-visual-1"]');
await activeEditor.click();
await activeEditor.click({ button: "right" });
await page.getByRole("menu", { name: "AI 文本操作" }).waitFor();
assert.equal(await page.getByRole("menuitem", { name: /续写/ }).count(), 1);
assert.equal(await page.getByRole("menuitem", { name: /固定为风格样本/ }).count(), 1);
assert.equal(await page.getByText("null", { exact: true }).count(), 0);
await page.screenshot({ path: resolve(screenshotDirectory, "ai-context-menu.png"), fullPage: true });
await page.keyboard.press("Escape");

await page.keyboard.press("Control+Shift+P");
await page.getByRole("heading", { name: "选择 AI 操作" }).waitFor();
await page.getByRole("searchbox", { name: "搜索 AI 命令" }).fill("续写");
assert.equal(await page.locator(".command-palette-item:visible").count(), 1);
await page.screenshot({ path: resolve(screenshotDirectory, "command-palette.png"), fullPage: true });
await page.keyboard.press("Escape");

await activeEditor.click();
await page.keyboard.press("Alt+1");
await page.getByRole("heading", { name: "确认即将发送的上下文" }).waitFor();
assert.ok(await page.locator(".context-item").count() >= 1);
assert.equal(await page.getByText("L4_STYLE_GLOBAL", { exact: true }).count(), 1);
assert.ok(await page.getByText("L2_STRUCTURAL", { exact: true }).count() >= 1);
assert.ok(await page.getByText("L3_KNOWLEDGE", { exact: true }).count() >= 2);
await page.getByText(/summary:project:project-visual-1@/).waitFor();
await page.getByText(/knowledge:fact:knowledge-visual-1@r0/).waitFor();
assert.equal(
  await page.locator(".context-exclusions code", { hasText: "knowledge:constraint:knowledge-visual-2@r0" }).count(),
  1,
);
await page.screenshot({ path: resolve(screenshotDirectory, "context-preview.png"), fullPage: true });
await page.getByRole("button", { name: "确认并发送" }).click();
await page.getByRole("heading", { name: "逐项审查 AI 修改" }).waitFor();
assert.equal(await page.locator(".review-hunk").count(), 1);
await page.screenshot({ path: resolve(screenshotDirectory, "patch-review.png"), fullPage: true });

await page.getByRole("button", { name: "全部接受", exact: true }).click();
await page.getByText(/已接受全部 1 个修改项/).waitFor();
await page.getByRole("button", { name: "应用已接受修改" }).click();
await page.getByText("雨停了。她推开车站的门。").waitFor();
await page.getByText(/ai_accept Commit/).waitFor();
await page.screenshot({ path: resolve(screenshotDirectory, "applied.png"), fullPage: true });

await page.setViewportSize({ width: 960, height: 700 });
assert.equal(await page.getByRole("button", { name: "关闭项目" }).isVisible(), true);
await page.screenshot({ path: resolve(screenshotDirectory, "minimum-width.png"), fullPage: true });

assert.deepEqual(errors, []);
await browser.close();
console.log(`Desktop visual flow passed; screenshots: ${screenshotDirectory}`);
