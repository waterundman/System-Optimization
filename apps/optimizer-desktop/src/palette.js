import { t } from "./i18n.js";
import { element, button } from "./dom.js";
import { state, AI_OPERATION_COMMANDS, runAiOperation } from "./main.js";
import { renderWorkspace, operationCommandsDisabled } from "./workspace.js";
import { pinCurrentStyleSample } from "./drawers.js";

function openAiContextMenu(x, y, blockId) {
  state.commandPaletteOpen = false;
  state.aiContextMenu = { x, y, blockId };
  renderWorkspace();
  queueMicrotask(() => document.querySelector(".ai-context-menu button:not(:disabled)")?.focus());
}

function aiContextMenuView() {
  const context = state.aiContextMenu;
  if (!context || !state.workspace) return null;
  const menu = element("div", {
    className: "ai-context-menu",
    attrs: { role: "menu", "aria-label": t("ai.menu.title") },
  }, [
    element("div", { className: "context-menu-heading", text: t("ai.menu.title") }),
    ...AI_OPERATION_COMMANDS.map((command) => commandMenuButton(command)),
    element("div", { className: "context-menu-separator", attrs: { role: "separator" } }),
    commandMenuButton({
      type: "pin_style",
      label: t("ai.menu.pinStyle"),
      description: t("ai.menu.saveSelection"),
      shortcut: "",
    }),
    element("div", { className: "context-menu-hint", text: t("ai.menu.hint") }),
  ]);
  menu.style.left = `${Math.max(8, Math.min(context.x, window.innerWidth - 244))}px`;
  menu.style.top = `${Math.max(8, Math.min(context.y, window.innerHeight - 340))}px`;
  menu.addEventListener("keydown", (event) => {
    if (event.key === "Escape") {
      event.preventDefault();
      closeCommandSurfaces();
      return;
    }
    if (!["ArrowDown", "ArrowUp"].includes(event.key)) return;
    event.preventDefault();
    const items = [...menu.querySelectorAll('button[role="menuitem"]:not(:disabled)')];
    const current = items.indexOf(document.activeElement);
    const delta = event.key === "ArrowDown" ? 1 : -1;
    items[(current + delta + items.length) % items.length]?.focus();
  });
  return menu;
}

function commandMenuButton(command) {
  const item = button("", "context-menu-command", () => executeRegisteredCommand(command.type), {
    disabled: operationCommandsDisabled(),
    attrs: {
      role: "menuitem",
      ...(command.shortcut ? { "aria-keyshortcuts": command.shortcut } : {}),
    },
  });
  item.append(element("span", { text: command.label }));
  if (command.shortcut) item.append(element("kbd", { text: command.shortcut }));
  return item;
}

function openCommandPalette() {
  if (!state.workspace || state.aiContextPreview) return;
  state.aiContextMenu = null;
  state.commandPaletteOpen = true;
  renderWorkspace();
  queueMicrotask(() => document.querySelector("#command-palette-query")?.focus());
}

function commandPaletteModal() {
  if (!state.commandPaletteOpen || !state.workspace) return null;
  // v0.8.0 Stage 3 (a11y): the palette is a real listbox so screen readers
  // announce the active option as the user arrows through it. The query
  // input owns the keyboard: ArrowUp/Down move the active option, Enter
  // executes it, Escape closes. aria-activedescendant on the listbox points
  // at the currently active option's id so AT speaks it without moving DOM
  // focus away from the input.
  let activeIndex = 0;
  const commands = [
    ...AI_OPERATION_COMMANDS,
    {
      type: "pin_style",
      label: t("ai.menu.pinStyle"),
      description: t("ai.menu.saveToLibrary"),
      shortcut: "",
    },
  ];
  const commandRows = commands.map((command, index) => {
    const row = button("", "command-palette-item", () => executeRegisteredCommand(command.type), {
      disabled: operationCommandsDisabled(),
      attrs: {
        "data-command-search": `${command.label} ${command.description} ${command.type}`.toLowerCase(),
        role: "option",
        id: `command-palette-option-${index}`,
        "aria-selected": index === 0 ? "true" : "false",
      },
    });
    row.append(element("span", { className: "command-palette-copy" }, [
      element("strong", { text: command.label }),
      element("span", { text: command.description }),
    ]));
    if (command.shortcut) row.append(element("kbd", { text: command.shortcut }));
    return row;
  });
  const list = element("div", {
    className: "command-palette-list",
    attrs: {
      id: "command-palette-list",
      role: "listbox",
      "aria-labelledby": "command-palette-title",
      "aria-activedescendant": commandRows[0] ? commandRows[0].id : "",
    },
  }, commandRows);
  const query = element("input", {
    type: "search",
    placeholder: t("ai.palette.searchPlaceholder"),
    attrs: {
      id: "command-palette-query",
      "aria-label": t("ai.palette.searchLabel"),
      "aria-controls": "command-palette-list",
      "aria-autocomplete": "list",
      "aria-expanded": "true",
      "aria-activedescendant": commandRows[0] ? commandRows[0].id : "",
      autocomplete: "off",
    },
  });
  const updateActive = (nextIndex) => {
    if (nextIndex < 0 || nextIndex >= commandRows.length) return;
    activeIndex = nextIndex;
    for (let i = 0; i < commandRows.length; i++) {
      commandRows[i].setAttribute("aria-selected", i === activeIndex ? "true" : "false");
    }
    const activeId = commandRows[activeIndex].id;
    list.setAttribute("aria-activedescendant", activeId);
    query.setAttribute("aria-activedescendant", activeId);
  };
  const visibleIndices = () => commandRows
    .map((row, i) => ({ row, i }))
    .filter(({ row }) => !row.hidden && !row.disabled)
    .map(({ i }) => i);
  query.addEventListener("input", () => {
    const needle = query.value.trim().toLowerCase();
    for (const row of commandRows) {
      row.hidden = !row.dataset.commandSearch.includes(needle);
    }
    const firstVisible = visibleIndices()[0];
    if (firstVisible !== undefined) updateActive(firstVisible);
  });
  query.addEventListener("keydown", (event) => {
    if (event.key === "Escape") {
      event.preventDefault();
      closeCommandSurfaces();
    } else if (event.key === "Enter") {
      event.preventDefault();
      const active = commandRows[activeIndex];
      if (active && !active.hidden && !active.disabled) active.click();
    } else if (event.key === "ArrowDown") {
      event.preventDefault();
      const visible = visibleIndices();
      if (!visible.length) return;
      const currentPos = visible.indexOf(activeIndex);
      const nextPos = currentPos < 0 ? 0 : (currentPos + 1) % visible.length;
      updateActive(visible[nextPos]);
    } else if (event.key === "ArrowUp") {
      event.preventDefault();
      const visible = visibleIndices();
      if (!visible.length) return;
      const currentPos = visible.indexOf(activeIndex);
      const prevPos = currentPos < 0
        ? 0
        : (currentPos - 1 + visible.length) % visible.length;
      updateActive(visible[prevPos]);
    }
  });
  const backdrop = element("div", {
    className: "command-palette-backdrop",
    attrs: { role: "presentation" },
  }, [
    element("section", {
      className: "command-palette",
      attrs: { role: "dialog", "aria-modal": "true", "aria-labelledby": "command-palette-title" },
    }, [
      element("div", { className: "command-palette-heading" }, [
        element("div", {}, [
          element("span", { className: "eyebrow", text: "COMMAND PALETTE" }),
          element("h2", { text: t("ai.palette.title"), attrs: { id: "command-palette-title" } }),
        ]),
        element("kbd", { text: "Esc" }),
      ]),
      query,
      operationCommandsDisabled()
        ? element("p", { className: "command-palette-warning", text: t("ai.palette.warning") })
        : null,
      list,
    ]),
  ]);
  backdrop.addEventListener("pointerdown", (event) => {
    if (event.target === backdrop) closeCommandSurfaces();
  });
  return backdrop;
}

function executeRegisteredCommand(commandType) {
  closeCommandSurfaces();
  if (commandType === "pin_style") {
    void pinCurrentStyleSample();
  } else {
    void runAiOperation(commandType);
  }
}

function closeCommandSurfaces() {
  state.aiContextMenu = null;
  state.commandPaletteOpen = false;
  document.querySelector(".ai-context-menu")?.remove();
  document.querySelector(".command-palette-backdrop")?.remove();
}

export {
  openAiContextMenu,
  aiContextMenuView,
  commandMenuButton,
  openCommandPalette,
  commandPaletteModal,
  executeRegisteredCommand,
  closeCommandSurfaces,
};
