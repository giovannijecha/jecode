import type { View } from "../../../src/tui/view.ts";
import type { LabState } from "../model.ts";
import * as edit from "../../../src/tui/editor.ts";
import { base } from "./shared.ts";
import { settingsPicker } from "../../../src/settings-command.ts";
import { permissionControlPicker } from "../../../src/permission-command.ts";
import { sessionPermissions } from "../../../src/permissions.ts";
import { builtinTools } from "../../../src/tools/index.ts";
import { workspace } from "../fixtures.ts";
import { heading } from "../../../src/tui/picker.ts";
import { providerLabel } from "../../../src/provider-label.ts";

export function providersScene(state: LabState): View {
  return {
    ...base(state), blocks: [], editor: edit.EMPTY, scroll: 0,
    modal: { kind: "pick", picker: {
      title: [],
      description: `Current route: ${providerLabel("anthropic")} · access changes do not switch it; use /models`,
      options: [
        { label: "Account", value: "1 provider" },
        { label: "API", value: `3 providers · current ${providerLabel("anthropic")}` },
      ],
      index: state.selected,
    } },
  };
}

export function providerAccountScene(state: LabState): View {
  return {
    ...base(state), blocks: [], editor: edit.EMPTY, scroll: 0,
    modal: { kind: "pick", picker: {
      title: heading("Account", "provider access", state.palette),
      options: [{ label: providerLabel("openai-codex"), value: "not connected" }], index: 0,
    } },
  };
}

export function providerApiScene(state: LabState): View {
  return {
    ...base(state), blocks: [], editor: edit.EMPTY, scroll: 0,
    modal: { kind: "pick", picker: {
      title: heading("API", "provider access", state.palette),
      options: [
        { label: providerLabel("anthropic"), value: "current · API key · session" },
        { label: providerLabel("openai"), value: "API key · missing" },
        { label: providerLabel("ollama"), value: "API key · missing" },
      ],
      index: state.selected,
    } },
  };
}

export function settingsScene(state: LabState): View {
  return {
    ...base(state),
    footer: {
      workspace: `${workspace} (main)`,
      provider: providerLabel("ollama"),
      model: "deepseek-v4-flash:0731",
      effort: "high",
    },
    blocks: [],
    editor: edit.EMPTY,
    scroll: 0,
    modal: {
      kind: "pick",
      picker: settingsPicker({
        provider: "ollama",
        model: "deepseek-v4-flash:0731",
        effort: "high",
        maxTokens: 64000,
        compactionPercent: 85,
        reducedMotion: false,
      }, state.selected),
    },
  };
}

export function permissionsScene(state: LabState): View {
  const control = permissionFixture();
  return {
    ...base(state),
    blocks: [],
    editor: edit.EMPTY,
    scroll: 0,
    modal: {
      kind: "pick",
      picker: permissionControlPicker(control, state.selected),
    },
  };
}

function permissionFixture() {
  const tools = builtinTools().map(({ name, description, input, dangerous, concurrency }) => ({
    name, description, input, dangerous, concurrency,
    async run(): Promise<never> { throw new Error("the TUI lab cannot execute tools"); },
  }));
  const control = sessionPermissions(tools);
  control.set("search_text", "deny");
  for (const path of ["fixture-one.ts", "fixture-two.ts"]) {
    control.remember({ kind: "tool_call", id: path, name: "edit_file", input: { path } });
  }
  control.remember({ kind: "tool_call", id: "fixture-command", name: "run_command", input: { command: "npm test" } });
  return control;
}
