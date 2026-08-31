// The session-local permission control plane exposed through /permissions.

import type { Host } from "./commands.ts";
import type { Session } from "./session.ts";
import type {
  PermissionGrant,
  PermissionMode,
  PermissionTool,
  SessionPermissions,
} from "./permissions.ts";
import type { Picker } from "./tui/picker.ts";
import { heading } from "./tui/picker.ts";
import type { Palette } from "./ui/theme.ts";

export async function permissionsCommand(session: Session, host: Host): Promise<void> {
  const choose = host.choose;
  const control = host.permissions;
  if (choose === undefined || control === undefined) {
    host.emit({ kind: "notice", text: "permissions need the interactive screen", tone: "warn" });
    return;
  }

  let selected = 0;
  while (true) {
    const tools = control.listTools();
    const index = await choose(permissionsPicker(tools, session.palette, selected));
    if (index === undefined) return;
    const tool = tools[index];
    if (tool === undefined) return;
    selected = index;
    await configureTool(tool, control, choose, session.palette);
  }
}

export function permissionsPicker(
  tools: readonly PermissionTool[],
  pal: Palette,
  index = 0,
): Picker {
  const launchOverride = tools.some((tool) => tool.locked);
  return {
    title: heading(
      "permissions",
      launchOverride ? "session only · auto approve at launch" : "session only",
      pal,
    ),
    description: "Changes apply now · /new resets them",
    options: tools.map((tool) => ({ label: tool.name, hint: toolHint(tool) })),
    index: Math.min(Math.max(0, index), Math.max(0, tools.length - 1)),
  };
}

async function configureTool(
  tool: PermissionTool,
  control: SessionPermissions,
  choose: NonNullable<Host["choose"]>,
  pal: Palette,
): Promise<void> {
  if (tool.locked) {
    await choose({
      title: heading(tool.name, "launch override", pal),
      description: "Restart without --auto-approve to change this tool",
      options: [{ label: "allow", hint: "locked for this process" }],
      index: 0,
    });
    return;
  }

  const modes: PermissionMode[] = tool.dangerous ? ["ask", "allow", "deny"] : ["allow", "deny"];
  const grants = control.listGrants(tool.name);
  const index = await choose({
    title: heading(tool.name, tool.dangerous ? "dangerous tool" : "read-only tool", pal),
    description: tool.dangerous
      ? "Session only · ask is the safe default"
      : "Session only · deny hides this tool from the model",
    options: [
      ...modes.map((mode) => ({ label: mode, hint: modeHint(mode, tool.dangerous) })),
      ...(grants.length === 0
        ? []
        : [{ label: "remembered approvals", hint: String(grants.length) }]),
    ],
    index: Math.max(0, modes.indexOf(tool.mode)),
  });
  if (index === undefined) return;
  const mode = modes[index];
  if (mode !== undefined) {
    control.set(tool.name, mode);
    return;
  }

  await reviewGrants(tool.name, control, choose, pal);
}

async function reviewGrants(
  tool: string,
  control: SessionPermissions,
  choose: NonNullable<Host["choose"]>,
  pal: Palette,
): Promise<void> {
  let selected = 0;
  while (true) {
    const grants = control.listGrants(tool);
    if (grants.length === 0) return;
    const options: Picker["options"] = [
      ...grants.map((grant) => ({ label: grant.label, hint: "revoke" })),
      ...(grants.length > 1 ? [{ label: "all remembered approvals", hint: "revoke all" }] : []),
    ];
    const index = await choose({
      title: heading("remembered", tool, pal),
      description: "Allowed without asking · this session",
      options,
      index: Math.min(selected, options.length - 1),
    });
    if (index === undefined) return;
    if (index === grants.length) {
      control.revokeTool(tool);
      return;
    }

    const grant: PermissionGrant | undefined = grants[index];
    if (grant === undefined) return;
    control.revoke(grant.key);
    selected = index;
  }
}

function toolHint(tool: PermissionTool): string {
  const kind = tool.dangerous ? "" : " · read only";
  const remembered = tool.remembered === 0 ? "" : ` · ${tool.remembered} remembered`;
  const locked = tool.locked ? " · launch override" : "";
  return `${tool.mode}${kind}${remembered}${locked}`;
}

function modeHint(mode: PermissionMode, dangerous: boolean): string {
  if (mode === "deny") return "hide from the model";
  if (mode === "ask") return "prompt when needed";
  return dangerous ? "every call this session" : "offer to the model";
}
