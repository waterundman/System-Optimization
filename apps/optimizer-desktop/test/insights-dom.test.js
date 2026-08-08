import assert from "node:assert/strict";
import test from "node:test";
import { parseHTML } from "linkedom";

// Set up minimal browser globals before importing main.js so its top-level
// `document.querySelector("#app")` and `window.addEventListener` calls do not
// throw. Each individual test creates its own linkedom document and passes it
// directly to `insightsDrawer` so the rendered tree is isolated from this
// bootstrap document.
const bootstrapDom = parseHTML(
  `<!DOCTYPE html><html><body><div id="app"></div></body></html>`,
);
global.document = bootstrapDom.document;
global.window = bootstrapDom.window;
if (!global.localStorage) {
  global.localStorage = {
    getItem: () => null,
    setItem: () => {},
    removeItem: () => {},
    clear: () => {},
  };
}
if (!global.crypto) {
  global.crypto = { subtle: {} };
}

const { insightsDrawer, state } = await import("../dist/main.js");

function createDocument() {
  const { document } = parseHTML(
    `<!DOCTYPE html><html><body></body></html>`,
  );
  return document;
}

function resetInsightsState() {
  state.insightsData = null;
  state.insightsBusy = null;
  state.showRevisionMetrics = false;
}

function sampleInsightsData(overrides = {}) {
  return {
    summary: {
      totalRuns: 5,
      totalInputTokens: 1200,
      totalOutputTokens: 800,
      totalTokens: 2000,
      acceptedCount: 3,
      rejectedCount: 1,
      conflictedCount: 1,
    },
    recentRuns: [
      {
        runId: "run-1",
        operationIntentId: "intent-1",
        state: "accepted",
        providerId: "deepseek",
        startedAt: "2026-07-15T00:00:00Z",
        totalTokens: 1000,
        inputTokens: 600,
        outputTokens: 400,
        cachedInputTokens: 100,
      },
      {
        runId: "run-2",
        operationIntentId: "intent-2",
        state: "rejected",
        providerId: "qwen",
        startedAt: "2026-07-15T01:00:00Z",
        totalTokens: 500,
        inputTokens: 400,
        outputTokens: 100,
        cachedInputTokens: 0,
      },
    ],
    generatedAt: "2026-07-15T02:00:00Z",
    revisionMetrics: {
      totalProposalsAccepted: 24,
      proposalsWithRejection: 8,
      acceptedAfterRejection: 3,
      acceptedAfterRejectionRate: 0.125,
    },
    payloadHashVariations: {
      totalProposals: 58,
      proposalsWithVariation: 3,
      variations: 4,
      variationRate: 0.05172413793103448,
    },
    ...overrides,
  };
}

test("T01: 空 insightsData 时 insightsDrawer 渲染 placeholder 元素", () => {
  resetInsightsState();
  state.insightsData = null;
  state.insightsBusy = null;
  const doc = createDocument();
  const el = insightsDrawer(doc);
  assert.ok(el, "insightsDrawer should return an element");
  assert.equal(el.tagName.toLowerCase(), "aside");
  assert.ok(
    el.textContent.includes("尚未加载数据"),
    `expected placeholder text, got: ${el.textContent}`,
  );
  // Empty state should not render summary cards
  assert.equal(el.querySelectorAll(".insights-summary-card").length, 0);
});

test("T02: 有 insightsData 时渲染 6 张摘要卡片且数值正确", () => {
  resetInsightsState();
  state.insightsData = sampleInsightsData();
  state.insightsBusy = null;
  const doc = createDocument();
  const el = insightsDrawer(doc);
  const cards = el.querySelectorAll(".insights-summary-card");
  assert.equal(cards.length, 6, "should render 6 summary cards");
  const labels = Array.from(cards).map((card) =>
    card.querySelector(".label")?.textContent,
  );
  assert.deepEqual(labels, [
    "总操作次数",
    "输入 Token 累计",
    "输出 Token 累计",
    "Token 总计",
    "接受率",
    "估算费用（USD）",
  ]);
  // totalRuns value
  assert.equal(cards[0].querySelector(".value").textContent, "5");
  // totalInputTokens value
  assert.equal(cards[1].querySelector(".value").textContent, "1,200");
  // totalOutputTokens value
  assert.equal(cards[2].querySelector(".value").textContent, "800");
  // totalTokens value
  assert.equal(cards[3].querySelector(".value").textContent, "2,000");
  // acceptRate: accepted=3, rejected=1, conflicted=1, decidedTotal=5, 3/5=60.0%
  assert.equal(cards[4].querySelector(".value").textContent, "60.0%");
});

test("T03: 有 recentRuns 时渲染费用明细表（含表头与每行 provider）", () => {
  resetInsightsState();
  state.insightsData = sampleInsightsData();
  state.insightsBusy = null;
  const doc = createDocument();
  const el = insightsDrawer(doc);
  const feeTable = el.querySelector(".insights-fee-table");
  assert.ok(feeTable, "fee table should be rendered when recentRuns exist");
  const header = feeTable.querySelector(".insights-fee-header");
  assert.ok(header, "fee table should have a header row");
  const headerCells = Array.from(header.querySelectorAll("span")).map(
    (cell) => cell.textContent,
  );
  assert.deepEqual(headerCells, [
    "Provider",
    "输入 Token",
    "输出 Token",
    "缓存 Token",
    "估算费用（USD）",
  ]);
  const dataRows = feeTable.querySelectorAll(".insights-fee-row:not(.insights-fee-header)");
  // recentRuns has two providers: deepseek and qwen
  assert.equal(dataRows.length, 2, "should render one row per provider");
  const providerIds = Array.from(dataRows).map(
    (row) => row.querySelector("span").textContent,
  );
  assert.ok(providerIds.includes("deepseek"));
  assert.ok(providerIds.includes("qwen"));
});

test("T04: 有 recentRuns 时渲染最近操作列表且包含状态 pill", () => {
  resetInsightsState();
  state.insightsData = sampleInsightsData();
  state.insightsBusy = null;
  const doc = createDocument();
  const el = insightsDrawer(doc);
  const runList = el.querySelector(".insights-run-list");
  assert.ok(runList, "should render a run list when recentRuns exist");
  const rows = runList.querySelectorAll(".insights-run-row");
  assert.equal(rows.length, 2, "should render one row per recent run");
  const firstPill = rows[0].querySelector(".state-pill");
  assert.ok(firstPill, "each run row should have a state pill");
  assert.equal(firstPill.textContent, "已接受");
  assert.ok(
    firstPill.className.includes("state-accepted"),
    `pill should carry state-accepted class, got: ${firstPill.className}`,
  );
  const secondPill = rows[1].querySelector(".state-pill");
  assert.equal(secondPill.textContent, "已拒绝");
  assert.ok(secondPill.className.includes("state-rejected"));
});

test("T05: insightsBusy=true 时按钮被禁用且刷新按钮显示刷新中", () => {
  resetInsightsState();
  state.insightsData = null;
  state.insightsBusy = true;
  const doc = createDocument();
  const el = insightsDrawer(doc);
  const actions = el.querySelector(".insights-actions");
  assert.ok(actions, "should render the actions container");
  const buttons = actions.querySelectorAll("button");
  assert.equal(buttons.length, 2, "should render export + refresh buttons");
  for (const btn of buttons) {
    assert.equal(
      btn.disabled,
      true,
      `button "${btn.textContent}" should be disabled while busy`,
    );
  }
  assert.equal(
    buttons[1].textContent,
    "刷新中…",
    `refresh button should show busy label, got: ${buttons[1].textContent}`,
  );
  // Empty state copy should reflect busy state
  assert.ok(
    el.textContent.includes("正在加载…"),
    `expected busy placeholder text, got: ${el.textContent}`,
  );
});

test("T06: insightsData 存在但 recentRuns 为空时渲染空状态文案", () => {
  resetInsightsState();
  state.insightsData = sampleInsightsData({ recentRuns: [] });
  state.insightsBusy = null;
  const doc = createDocument();
  const el = insightsDrawer(doc);
  // Summary cards should still render
  assert.equal(
    el.querySelectorAll(".insights-summary-card").length,
    6,
    "summary cards should render even without recentRuns",
  );
  // Fee table should NOT render (no fee entries)
  assert.equal(
    el.querySelector(".insights-fee-table"),
    null,
    "fee table should be omitted when there are no fee entries",
  );
  // Run list should be absent; empty hint should be present
  assert.equal(
    el.querySelector(".insights-run-list"),
    null,
    "run list should be omitted when recentRuns is empty",
  );
  assert.ok(
    el.textContent.includes("暂无最近操作记录。"),
    `expected empty recent runs hint, got: ${el.textContent}`,
  );
});

test("T07: showRevisionMetrics 默认为 false 时二次编辑率卡片不渲染", () => {
  resetInsightsState();
  state.insightsData = sampleInsightsData();
  state.insightsBusy = null;
  state.showRevisionMetrics = false;
  const doc = createDocument();
  const el = insightsDrawer(doc);
  // D5 约束：指标 UI 默认隐藏，避免"看到历史改变行为"偏差
  assert.equal(
    el.querySelector(".insights-revision-card"),
    null,
    "revision metrics card must NOT render when showRevisionMetrics is false",
  );
  // 切换开关按钮应存在（用户主动开启入口）
  const toggleBtn = el.querySelector(".insights-revision-actions button");
  assert.ok(toggleBtn, "toggle button should be present so users can opt in");
  assert.equal(
    toggleBtn.textContent,
    "显示二次编辑率",
    `toggle should read "显示二次编辑率" when hidden, got: ${toggleBtn.textContent}`,
  );
});

test("T08: showRevisionMetrics=true 时渲染二次编辑率卡片且数值正确", () => {
  resetInsightsState();
  state.insightsData = sampleInsightsData();
  state.insightsBusy = null;
  state.showRevisionMetrics = true;
  const doc = createDocument();
  const el = insightsDrawer(doc);
  const card = el.querySelector(".insights-revision-card");
  assert.ok(card, "revision metrics card should render when showRevisionMetrics is true");
  const label = card.querySelector(".label")?.textContent;
  assert.equal(label, "二次编辑率");
  // rate = 0.125 → 12.5% · 3/24
  const value = card.querySelector(".value")?.textContent;
  assert.equal(
    value,
    "12.5% · 3/24",
    `value should show "12.5% · 3/24", got: ${value}`,
  );
  // 切换按钮应变为"隐藏二次编辑率"
  const toggleBtn = el.querySelector(".insights-revision-actions button");
  assert.equal(
    toggleBtn.textContent,
    "隐藏二次编辑率",
    `toggle should read "隐藏二次编辑率" when visible, got: ${toggleBtn.textContent}`,
  );
});

test("T09: rate 颜色编码按低/中/高三档 hex 渲染", () => {
  const cases = [
    { rate: 0.05, hex: "#10b981", tier: "低 (<10%)" },
    { rate: 0.2, hex: "#eab308", tier: "中 (10-25%)" },
    { rate: 0.3, hex: "#ef4444", tier: "高 (>25%)" },
  ];
  for (const { rate, hex, tier } of cases) {
    resetInsightsState();
    state.insightsData = sampleInsightsData({
      revisionMetrics: {
        totalProposalsAccepted: 20,
        proposalsWithRejection: 10,
        acceptedAfterRejection: Math.round(rate * 20),
        acceptedAfterRejectionRate: rate,
      },
    });
    state.insightsBusy = null;
    state.showRevisionMetrics = true;
    const doc = createDocument();
    const el = insightsDrawer(doc);
    const valueEl = el.querySelector(".insights-revision-card .value");
    assert.ok(valueEl, `value element should exist for tier ${tier}`);
    const style = valueEl.getAttribute("style") || "";
    assert.ok(
      style.includes(hex),
      `tier ${tier} (rate=${rate}) should use color ${hex}, got style: "${style}"`,
    );
  }
});

test("T10: showRevisionMetrics 默认为 false 时 payload 变形率卡片不渲染", () => {
  // v0.7.0 Stage 3 (D5): payload 变形率卡片与二次编辑率卡片共用
  // showRevisionMetrics 开关，默认隐藏，避免"看到历史改变行为"偏差。
  resetInsightsState();
  state.insightsData = sampleInsightsData();
  state.insightsBusy = null;
  state.showRevisionMetrics = false;
  const doc = createDocument();
  const el = insightsDrawer(doc);
  // payload 变形率卡片在开关关闭时不应渲染
  assert.equal(
    el.querySelector(".insights-payload-card"),
    null,
    "payload hash variations card must NOT render when showRevisionMetrics is false",
  );
  // 开关按钮应存在（与二次编辑率共用同一入口）
  const toggleBtn = el.querySelector(".insights-revision-actions button");
  assert.ok(toggleBtn, "toggle button should be present so users can opt in");
});

test("T11: showRevisionMetrics=true 时渲染 payload 变形率卡片且数值正确", () => {
  // v0.7.0 Stage 3 (D4): showRevisionMetrics=true 时同时显示二次编辑率卡片
  // 和 payload 变形率卡片，不新增开关。
  resetInsightsState();
  state.insightsData = sampleInsightsData();
  state.insightsBusy = null;
  state.showRevisionMetrics = true;
  const doc = createDocument();
  const el = insightsDrawer(doc);
  const card = el.querySelector(".insights-payload-card");
  assert.ok(
    card,
    "payload hash variations card should render when showRevisionMetrics is true",
  );
  const label = card.querySelector(".label")?.textContent;
  assert.equal(label, "payload 变形率");
  // sampleInsightsData: variationRate ≈ 0.0517 → 5.2% · 3/58
  const value = card.querySelector(".value")?.textContent;
  assert.equal(
    value,
    "5.2% · 3/58",
    `value should show "5.2% · 3/58", got: ${value}`,
  );
  // 二次编辑率卡片应同时存在（共用开关）
  const revisionCard = el.querySelector(".insights-revision-card");
  assert.ok(
    revisionCard,
    "revision metrics card should also render when showRevisionMetrics is true",
  );
  // 切换按钮应仍为"隐藏二次编辑率"（共用开关文案）
  const toggleBtn = el.querySelector(".insights-revision-actions button");
  assert.equal(
    toggleBtn.textContent,
    "隐藏二次编辑率",
    `toggle should read "隐藏二次编辑率" when visible, got: ${toggleBtn.textContent}`,
  );
});

test("T12: payload 变形率 rate 颜色编码按低/中/高三档 hex 渲染", () => {
  // v0.7.0 Stage 3 (D4): rate 颜色编码用 hex（参考 v0.5.0 fpsMonitor 模式）。
  // 阈值：低(绿 #10b981, <5%)、中(黄 #eab308, 5-15%)、高(红 #ef4444, >15%)。
  const cases = [
    { rate: 0.03, hex: "#10b981", tier: "低 (<5%)" },
    { rate: 0.1, hex: "#eab308", tier: "中 (5-15%)" },
    { rate: 0.2, hex: "#ef4444", tier: "高 (>15%)" },
  ];
  for (const { rate, hex, tier } of cases) {
    resetInsightsState();
    state.insightsData = sampleInsightsData({
      payloadHashVariations: {
        totalProposals: 50,
        proposalsWithVariation: Math.round(rate * 50),
        variations: Math.round(rate * 50),
        variationRate: rate,
      },
    });
    state.insightsBusy = null;
    state.showRevisionMetrics = true;
    const doc = createDocument();
    const el = insightsDrawer(doc);
    const valueEl = el.querySelector(".insights-payload-card .value");
    assert.ok(valueEl, `value element should exist for tier ${tier}`);
    const style = valueEl.getAttribute("style") || "";
    assert.ok(
      style.includes(hex),
      `tier ${tier} (rate=${rate}) should use color ${hex}, got style: "${style}"`,
    );
  }
});
