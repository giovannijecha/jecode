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
    const index = await choose(permissionControlPicker(control, host, selected));
    if (index === undefined) return;
    const tool = control.listTools()[index];
    if (tool === undefined) return;
    selected = index;
    if (tool.locked) {
      lockedNotice(tool.name, host);
      continue;
    }
    if (tool.remembered > 0) {
      await reviewGrants(tool.name, control, choose, session.palette);
    }
  }
}

export function permissionsPicker(
  tools: readonly PermissionTool[],
  index = 0,
  adjust?: NonNullable<Picker["adjust"]>,
): Picker {
  return {
    title: [],
    options: tools.map((tool) => {
      const description = toolDescription(tool);
      return {
        label: tool.name,
        ...(description === undefined ? {} : { description }),
        value: tool.locked ? `${tool.mode} · locked` : tool.mode,
        adjustable: !tool.locked,
      };
    }),
    visible: tools.length,
    ...(adjust === undefined ? {} : { adjust }),
    index: Math.min(Math.max(0, index), Math.max(0, tools.length - 1)),
  };
}

function permissionControlPicker(
  control: SessionPermissions,
  host: Host,
  selected: number,
): Picker {
  return permissionsPicker(control.listTools(), selected, (index, step) => {
    const tool = control.listTools()[index];
    if (tool === undefined) return permissionControlPicker(control, host, selected);
    if (tool.locked) {
      lockedNotice(tool.name, host);
      return permissionControlPicker(control, host, index);
    }

    const modes = modesFor(tool);
    const at = Math.max(0, modes.indexOf(tool.mode));
    const next = modes[(at + step + modes.length) % modes.length];
    if (next !== undefined) control.set(tool.name, next);
    return permissionControlPicker(control, host, index);
  });
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

function modesFor(tool: PermissionTool): readonly PermissionMode[] {
  return tool.dangerous ? ["ask", "allow", "deny"] : ["allow", "deny"];
}

function toolDescription(tool: PermissionTool): string | undefined {
  const parts = [
    tool.dangerous ? undefined : "read only",
    tool.remembered === 0 ? undefined : `${tool.remembered} remembered`,
  ].filter((part): part is string => part !== undefined);
  return parts.length === 0 ? undefined : parts.join(" · ");
}

function lockedNotice(tool: string, host: Host): void {
  host.emit({
    kind: "notice",
    text: `restart without --auto-approve to change ${tool}`,
    tone: "warn",
  });
}
