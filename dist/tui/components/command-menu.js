import { menuWindow, renderMenuRows } from "./menu.js";
const MAX_ROWS = 4;
export function renderCommandMenu(commands, selected, width, pal) {
    const selectedIndex = selected === undefined ? 0 : Math.max(0, Math.min(commands.length - 1, selected));
    const { first, last } = menuWindow(commands.length, selectedIndex, MAX_ROWS);
    const shown = commands.slice(first, last);
    if (shown.length === 0)
        return [];
    return renderMenuRows(shown.map((command, index) => ({
        label: `/${command.name}`,
        description: command.blurb,
        selected: selectedIndex === first + index,
    })), width, pal);
}
export function commandMenuLimit() {
    return MAX_ROWS;
}
