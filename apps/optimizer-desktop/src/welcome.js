import { t } from "./i18n.js";
import { element, button, labeledInput, directoryField, formValue, setFormBusy } from "./dom.js";
import { invokeHost } from "./client.js";
import { normalizeHostError } from "./frontend-state.js";
import { state, app, setNotice, noticeView, loadWorkspace } from "./main.js";
import { disposeVirtualControllers } from "./workspace.js";

// v0.8.0 无边框窗口：把最小化/最大化/关闭融进前端自定义标题栏。
// 依赖 Tauri 的 core:window 权限（start-dragging / minimize / toggle-maximize / close）。
function setupWindowControls() {
  const appWindow = window.__TAURI__?.window?.getCurrentWindow?.();
  if (!appWindow) return;
  const labels = {
    minimize: t("window.minimize"),
    maximize: t("window.maximize"),
    close: t("window.close"),
  };
  const applyAction = (action) => {
    if (action === "minimize") {
      appWindow.minimize();
    } else if (action === "maximize") {
      appWindow.toggleMaximize();
    } else if (action === "close") {
      appWindow.close();
    }
  };
  document.querySelectorAll("[data-win]").forEach((button) => {
    const action = button.getAttribute("data-win");
    button.removeAttribute("aria-hidden");
    button.setAttribute("aria-label", labels[action] || action);
    button.title = labels[action] || action;
    button.addEventListener("click", () => applyAction(action));
  });
}

function renderWelcome() {
  disposeVirtualControllers();
  const createForm = element("form", { className: "create-form" }, [
    labeledInput(t("welcome.create.titleLabel"), "title", t("welcome.create.titlePlaceholder"), true),
    directoryField(t("welcome.create.saveTo"), "parentDirectory", t("welcome.create.saveToPlaceholder")),
    labeledInput(t("welcome.create.folderLabel"), "folderName", t("welcome.create.folderPlaceholder"), true),
    element("p", { className: "form-hint", text: t("welcome.create.hint") }),
    element("button", { className: "primary-button create-form-submit", text: t("welcome.create.submit"), type: "submit" }),
  ]);
  createForm.addEventListener("submit", createProject);

  const openForm = element("form", { className: "open-form" }, [
    labeledInput(t("welcome.open.pathLabel"), "projectDirectory", t("welcome.open.pathPlaceholder"), true),
    element("p", { className: "form-hint", text: t("welcome.open.hint") }),
    element("button", { className: "secondary-button open-form-submit", text: t("welcome.open.submit"), type: "submit" }),
  ]);
  openForm.addEventListener("submit", openProject);

  app.replaceChildren(element("div", { className: "welcome-container" }, [
    element("div", { className: "welcome-header" }, [
      element("div", { className: "welcome-brand" }, [
        element("div", { className: "brand-mark brand-mark-large", text: t("common.brandMark") }),
      ]),
    ]),
    element("div", { className: "welcome-content" }, [
      element("div", { className: "welcome-main" }, [
        element("div", { className: "welcome-block welcome-block-create" }, [
          element("div", { className: "welcome-block-header" }, [
            element("h2", { text: t("welcome.create.title") }),
          ]),
          createForm,
        ]),
      ]),
      element("div", { className: "welcome-sidebar" }, [
        element("div", { className: "welcome-block welcome-block-open" }, [
          element("div", { className: "welcome-block-header" }, [
            element("h2", { text: t("welcome.open.title") }),
          ]),
          openForm,
        ]),
        recentProjectsView(),
      ]),
    ]),
    noticeView(),
  ]));
}

async function loadRecentProjects() {
  try {
    state.recentProjects = await invokeHost("list_recent_projects");
  } catch (error) {
    state.recentProjects = [];
    setNotice("warning", normalizeHostError(error).message);
  }
}

function recentProjectsView() {
  if (!state.recentProjects.length) return null;
  return element("section", { className: "recent-projects", attrs: { "aria-labelledby": "recent-projects-title" } }, [
    element("div", { className: "recent-projects-heading" }, [
      element("h2", { text: t("recent.title"), attrs: { id: "recent-projects-title" } }),
      element("span", { text: t("recent.count", { count: state.recentProjects.length }) }),
    ]),
    ...state.recentProjects.map(recentProjectRow),
  ]);
}

function recentProjectRow(project) {
  const busy = state.recentProjectBusy === project.projectId;
  const open = button("", "recent-project-open", () => openRecentProject(project), {
    disabled: busy || !project.available,
    attrs: { "aria-label": t("recent.openLabel", { title: project.title }) },
  });
  open.append(
    element("span", { className: "recent-project-mark", text: t("common.brandMark") }),
    element("span", { className: "recent-project-copy" }, [
      element("strong", { text: project.title }),
      element("span", { text: project.available ? project.directory : t("recent.unavailable") }),
    ]),
    element("span", { className: "recent-project-time", text: formatRecentTime(project.lastOpenedAt) }),
  );
  return element("div", { className: `recent-project${project.available ? "" : " recent-project-missing"}` }, [
    open,
    button(t("recent.remove"), "recent-project-remove", () => removeRecentProject(project), {
      disabled: busy,
      attrs: { "aria-label": t("recent.removeLabel", { title: project.title }) },
    }),
  ]);
}

function formatRecentTime(value) {
  const date = new Date(value);
  if (Number.isNaN(date.getTime())) return t("recent.lastOpened");
  return new Intl.DateTimeFormat("zh-CN", { month: "numeric", day: "numeric" }).format(date);
}

async function openRecentProject(project) {
  if (!project.available || state.recentProjectBusy) return;
  state.recentProjectBusy = project.projectId;
  renderWelcome();
  try {
    state.session = await invokeHost("open_recent_project", {
      input: { schemaVersion: 1, projectId: project.projectId },
    });
    state.recentProjectBusy = null;
    setNotice(null, null);
    await loadWorkspace();
  } catch (error) {
    state.recentProjectBusy = null;
    setNotice("error", normalizeHostError(error).message);
    await loadRecentProjects();
    renderWelcome();
  }
}

async function removeRecentProject(project) {
  if (state.recentProjectBusy) return;
  state.recentProjectBusy = project.projectId;
  renderWelcome();
  try {
    await invokeHost("remove_recent_project", {
      input: { schemaVersion: 1, projectId: project.projectId },
    });
    state.recentProjects = state.recentProjects.filter((item) => item.projectId !== project.projectId);
    setNotice("success", t("recent.removed", { title: project.title }));
  } catch (error) {
    setNotice("error", normalizeHostError(error).message);
  } finally {
    state.recentProjectBusy = null;
    renderWelcome();
  }
}

async function createProject(event) {
  event.preventDefault();
  const form = event.currentTarget;
  const input = {
    schemaVersion: 1,
    parentDirectory: formValue(form, "parentDirectory"),
    folderName: formValue(form, "folderName"),
    title: formValue(form, "title"),
    language: state.locale || "zh-CN",
  };
  setFormBusy(form, true);
  try {
    state.session = await invokeHost("create_project", { input });
    setNotice(null, null);
    await loadWorkspace();
  } catch (error) {
    setNotice("error", normalizeHostError(error).message);
    renderWelcome();
  } finally {
    setFormBusy(form, false);
  }
}

async function openProject(event) {
  event.preventDefault();
  const form = event.currentTarget;
  const input = {
    schemaVersion: 1,
    projectDirectory: formValue(form, "projectDirectory"),
  };
  setFormBusy(form, true);
  try {
    state.session = await invokeHost("open_project", { input });
    setNotice(null, null);
    await loadWorkspace();
  } catch (error) {
    setNotice("error", normalizeHostError(error).message);
    renderWelcome();
  } finally {
    setFormBusy(form, false);
  }
}

export {
  setupWindowControls,
  renderWelcome,
  loadRecentProjects,
  recentProjectsView,
  recentProjectRow,
  formatRecentTime,
  openRecentProject,
  removeRecentProject,
  createProject,
  openProject,
};
