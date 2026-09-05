import { test } from "node:test";
import assert from "node:assert/strict";
import { STEEL } from "../src/ui/theme.ts";

const ESC = String.fromCharCode(27);
const ANSI = new RegExp(`${ESC}\\[[0-9;]*m`, "g");

test("Ribbon uses a full-width selected dot band while writable prompts keep their arrow", async () => {
  const descriptor = Object.getOwnPropertyDescriptor(process.stdout, "isTTY");
  const noColor = process.env["NO_COLOR"];
  const term = process.env["TERM"];

  try {
    Object.defineProperty(process.stdout, "isTTY", { configurable: true, value: true });
    delete process.env["NO_COLOR"];
    process.env["TERM"] = "xterm-256color";

    const { renderCommandMenu } = await import("../src/tui/components/command-menu.ts");
    const { renderMenuRows, renderMenu } = await import("../src/tui/components/menu.ts");
    const { promptLine } = await import("../src/tui/components/prompt.ts");
    const picker = await import("../src/tui/picker.ts");
    const { configureColor } = await import("../src/ui/render.ts");
    const accent = `${ESC}[38;2;${STEEL.accent.join(";")}m`;
    const surface = `${ESC}[48;2;${STEEL.surface.subtle.join(";")}m`;

    const commandRows = renderCommandMenu([
      { name: "help", blurb: "show keyboard controls" },
      { name: "exit", blurb: "exit" },
    ], 0, 60, STEEL).rows;
    assert.ok(commandRows[0]?.includes(surface));
    assert.ok(commandRows.slice(1).every((row) => !row.includes(surface)));

    const commands = plain(commandRows);
    assert.match(commands[0] ?? "", /^● \/help/);
    assert.match(commands[1] ?? "", /^  \/exit/);
    assert.equal(commands[0]?.length, 60, "the selected band fills the menu width");
    assert.match(commands.slice(2).join("\n"), /show keyboard controls/);
    assert.doesNotMatch(commands[0] ?? "", /show keyboard controls/);

    const settings = plain(picker.panel({
      title: [],
      options: [{ label: "provider", hint: "ollama" }, { label: "effort", hint: "max" }],
      index: 0,
    }, 60, STEEL));
    assert.match(settings.find((row) => row.includes("provider")) ?? "", /^● provider/);
    assert.match(settings.find((row) => row.includes("effort")) ?? "", /^  effort/);

    const adjustable = plain(renderMenu([
      {
        label: "run_command",
        description: "1 remembered",
        value: "ask",
        adjustable: true,
        selected: true,
      },
    ], 60, STEEL, { maxRows: 6 }).rows);
    assert.match(adjustable[0] ?? "", /^● run_command.*‹ ask ›/);
    assert.doesNotMatch(adjustable[0] ?? "", /remembered/);
    assert.match(adjustable.slice(1).join("\n"), /1 remembered/);

    const narrow = plain(renderMenuRows([
      {
        label: "run_command",
        description: "1 remembered",
        value: "deny",
        adjustable: true,
        selected: true,
      },
    ], 32, STEEL));
    assert.match(narrow[0] ?? "", /^● run_command.*‹ deny ›/);
    assert.doesNotMatch(narrow[0] ?? "", /remembered/);

    const essentialRows = renderMenuRows([
      { label: "gpt-5.6-terra", value: "OpenAI Account", selected: true },
      { label: "claude-sonnet-5", value: "Anthropic API", selected: false },
    ], 38, STEEL);
    const essential = plain(essentialRows);
    assert.match(essential[0] ?? "", /^● gpt-5\.6-terra.*OpenAI Account/);
    assert.match(essential[1] ?? "", /^  claude-sonnet-5.*Anthropic API/);
    assert.ok(essentialRows[0]?.includes(surface));
    assert.ok(essentialRows[0]?.includes(accent));

    const input = plain([promptLine("secret", 6, 60, STEEL).row]);
    assert.match(input[0] ?? "", /^→ secret/);

    const empty = plain(picker.panel({
      title: [],
      options: [{ label: "alpha" }],
      searchable: true,
      query: "missing",
      index: 0,
    }, 60, STEEL));
    assert.match(empty.join("\n"), /no matches/i);

    configureColor(false);
    const monochrome = renderMenuRows([
      { label: "selected", selected: true },
      { label: "other", selected: false },
    ], 60, STEEL);
    assert.match(monochrome[0] ?? "", /^● selected/);
    assert.match(monochrome[1] ?? "", /^  other/);
    assert.equal(monochrome[0]?.length, 60);
    assert.ok(monochrome.every((row) => !row.includes(ESC)));
  } finally {
    if (descriptor === undefined) delete (process.stdout as { isTTY?: boolean }).isTTY;
    else Object.defineProperty(process.stdout, "isTTY", descriptor);
    if (noColor === undefined) delete process.env["NO_COLOR"];
    else process.env["NO_COLOR"] = noColor;
    if (term === undefined) delete process.env["TERM"];
    else process.env["TERM"] = term;
  }
});

test("menu details reserve only the rows required by their content", async () => {
  const { renderMenu } = await import("../src/tui/components/menu.ts");
  for (const description of [undefined, "", "One line."]) {
    const rows = plain(renderMenu([{ label: "Choice", description, selected: true }], 80, STEEL, { maxRows: 6 }).rows);
    assert.equal(rows.length, description ? 2 : 1);
    assert.ok(rows.every((row) => row.trim() !== ""));
  }
});

test("wrapped details keep a stable height across visible and offscreen choices", async () => {
  const { renderMenu } = await import("../src/tui/components/menu.ts");
  const { textWidth } = await import("../src/ui/width.ts");
  const entries = [
    { label: "First", description: "One line." },
    { label: "Second", description: "Another line." },
    { label: "Last", description: "界".repeat(20) },
  ];
  for (const width of [38, 80]) for (const selected of [0, 1, 2]) {
    const choices = entries.map((entry, index) => ({ ...entry, selected: index === selected }));
    const rendered = renderMenu(choices, width, STEEL, { maxRows: 6, visible: 2 });
    const rows = plain(rendered.rows);
    assert.equal(rows.length, width === 38 ? 4 : 3);
    assert.ok(rows.every((row) => textWidth(row) <= width));
    if (selected === 2) assert.equal(rows.slice(2).join("").replace(/\s/gu, ""), entries[2]!.description);
    const measured = renderMenu(choices, width, undefined, { maxRows: 6, visible: 2 });
    assert.equal(measured.first, rendered.first);
    assert.equal(measured.last, rendered.last);
  }
});

function plain(rows: readonly string[]): string[] {
  return rows.map((row) => row.replace(ANSI, ""));
}
