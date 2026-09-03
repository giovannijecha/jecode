import { test } from "node:test";
import assert from "node:assert/strict";
import { STEEL } from "../src/ui/theme.ts";

const ESC = String.fromCharCode(27);
const ANSI = new RegExp(`${ESC}\\[[0-9;]*m`, "g");

test("colour menu rows align with the composer while writable prompts keep their arrow", async () => {
  const descriptor = Object.getOwnPropertyDescriptor(process.stdout, "isTTY");
  const noColor = process.env["NO_COLOR"];
  const term = process.env["TERM"];

  try {
    Object.defineProperty(process.stdout, "isTTY", { configurable: true, value: true });
    delete process.env["NO_COLOR"];
    process.env["TERM"] = "xterm-256color";

    const { renderCommandMenu } = await import("../src/tui/components/command-menu.ts");
    const { renderMenuRows } = await import("../src/tui/components/menu.ts");
    const { promptLine } = await import("../src/tui/components/prompt.ts");
    const picker = await import("../src/tui/picker.ts");
    const { configureColor } = await import("../src/ui/render.ts");
    const accent = `${ESC}[38;2;${STEEL.accent.join(";")}m`;

    const commandRows = renderCommandMenu([
      { name: "help", blurb: "show keyboard controls" },
      { name: "exit", blurb: "exit" },
    ], 0, 60, STEEL).rows;
    assert.ok(commandRows.every((row) => !row.includes("\x1b[48;2;")));
    assert.ok((commandRows[0] ?? "").includes(`${accent}/help`));
    assert.ok(!(commandRows[1] ?? "").includes(`${accent}/exit`));

    const commands = plain(commandRows);
    assert.match(commands[0] ?? "", /^\/help/);
    assert.match(commands[1] ?? "", /^\/exit/);

    const settings = plain(picker.panel({
      title: [],
      options: [{ label: "provider", hint: "ollama" }, { label: "effort", hint: "max" }],
      index: 0,
    }, 60, STEEL));
    assert.match(settings.find((row) => row.includes("provider")) ?? "", /^provider/);
    assert.match(settings.find((row) => row.includes("effort")) ?? "", /^effort/);

    const adjustable = plain(renderMenuRows([
      {
        label: "run_command",
        description: "1 remembered",
        value: "ask",
        adjustable: true,
        selected: true,
      },
    ], 60, STEEL));
    assert.match(adjustable[0] ?? "", /^run_command.*1 remembered.*‹ ask ›/);

    const narrow = plain(renderMenuRows([
      {
        label: "run_command",
        description: "1 remembered",
        value: "deny",
        adjustable: true,
        selected: true,
      },
    ], 32, STEEL));
    assert.match(narrow[0] ?? "", /^run_command.*‹ deny ›/);
    assert.doesNotMatch(narrow[0] ?? "", /remembered/);

    const essentialRows = renderMenuRows([
      { label: "gpt-5.6-terra", value: "ChatGPT", selected: true },
      { label: "claude-sonnet-5", value: "Anthropic", selected: false },
    ], 38, STEEL);
    const essential = plain(essentialRows);
    assert.match(essential[0] ?? "", /^gpt-5\.6-terra.*ChatGPT/);
    assert.match(essential[1] ?? "", /^claude-sonnet-5.*Anthropic/);
    assert.ok((essentialRows[0] ?? "").includes(`${ESC}[1m${accent}ChatGPT`));

    const input = plain([promptLine("secret", 6, 60, STEEL).row]);
    assert.match(input[0] ?? "", /^→ secret/);

    const empty = plain(picker.panel({
      title: [],
      options: [{ label: "alpha" }],
      searchable: true,
      query: "missing",
      index: 0,
    }, 60, STEEL));
    assert.equal(empty.find((row) => row.includes("no matches")), "no matches");

    configureColor(false);
    const monochrome = renderMenuRows([
      { label: "selected", selected: true },
      { label: "other", selected: false },
    ], 60, STEEL);
    assert.match(monochrome[0] ?? "", /^→ selected/);
    assert.match(monochrome[1] ?? "", /^  other/);
  } finally {
    if (descriptor === undefined) delete (process.stdout as { isTTY?: boolean }).isTTY;
    else Object.defineProperty(process.stdout, "isTTY", descriptor);
    if (noColor === undefined) delete process.env["NO_COLOR"];
    else process.env["NO_COLOR"] = noColor;
    if (term === undefined) delete process.env["TERM"];
    else process.env["TERM"] = term;
  }
});

function plain(rows: readonly string[]): string[] {
  return rows.map((row) => row.replace(ANSI, ""));
}
