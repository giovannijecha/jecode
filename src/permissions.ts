// Session-local tool policies and remembered approval scopes.

import type { ToolCallBlock } from "./types.ts";
import type { Tool } from "./tools/index.ts";

export type PermissionMode = "allow" | "ask" | "deny";

export type PermissionScope = {
  key: string;
  label: string;
  summary: string;
};

export type PermissionGrant = {
  key: string;
  tools: readonly string[];
  label: string;
};

export type PermissionTool = {
  name: string;
  dangerous: boolean;
  mode: PermissionMode;
  remembered: number;
};

export type SessionPermissions = {
  listTools(): PermissionTool[];
  set(tool: string, mode: PermissionMode): boolean;
  listGrants(tool?: string): PermissionGrant[];
  revoke(key: string): void;
  revokeTool(tool: string): void;
  reset(): void;
  availableTools(): Tool[];
  /** Recheck revocation before a previously advertised call starts. */
  allowed(call: ToolCallBlock): boolean;
  approved(call: ToolCallBlock): boolean;
  remember(call: ToolCallBlock): void;
};

/** One permission control plane for one interactive process. */
export function sessionPermissions(
  tools: readonly Tool[],
): SessionPermissions {
  const catalogue = [...tools];
  const byName = new Map(catalogue.map((tool) => [tool.name, tool]));
  const modes = new Map<string, PermissionMode>();
  const grants = new Map<string, PermissionGrant>();

  const configured = (tool: Tool): PermissionMode =>
    modes.get(tool.name) ?? defaultMode(tool);

  const revokeTool = (name: string): void => {
    for (const [key, grant] of grants) {
      if (grant.tools.includes(name)) grants.delete(key);
    }
  };

  return {
    listTools() {
      return catalogue.map((tool) => ({
        name: tool.name,
        dangerous: tool.dangerous,
        mode: configured(tool),
        remembered: [...grants.values()].filter((grant) => grant.tools.includes(tool.name)).length,
      }));
    },

    set(name, mode) {
      const tool = byName.get(name);
      if (tool === undefined) return false;
      if (!tool.dangerous && mode === "ask") return false;
      if (configured(tool) === mode) return true;

      if (mode === defaultMode(tool)) modes.delete(name);
      else modes.set(name, mode);
      revokeTool(name);
      return true;
    },

    listGrants(name) {
      return [...grants.values()].filter((grant) => name === undefined || grant.tools.includes(name));
    },

    revoke(key) {
      grants.delete(key);
    },

    revokeTool,

    reset() {
      modes.clear();
      grants.clear();
    },

    availableTools() {
      return catalogue.filter((tool) => configured(tool) !== "deny");
    },

    allowed(call) {
      const tool = byName.get(call.name);
      return tool !== undefined && configured(tool) !== "deny";
    },

    approved(call) {
      const tool = byName.get(call.name);
      if (tool === undefined) return false;
      const mode = configured(tool);
      if (mode === "allow") return true;
      if (mode === "deny") return false;
      return grants.has(scopeFor(call).key);
    },

    remember(call) {
      const tool = byName.get(call.name);
      if (tool === undefined || configured(tool) !== "ask") return;
      const scope = scopeFor(call);
      grants.set(scope.key, { key: scope.key, tools: grantTools(call), label: scope.summary });
    },
  };
}

/** The narrow permission represented by "for the session" in an approval. */
export function scopeFor(call: ToolCallBlock): PermissionScope {
  const path = typeof call.input.path === "string" ? call.input.path : undefined;
  if ((call.name === "write_file" || call.name === "edit_file") && path !== undefined) {
    return { key: `file\0${path}`, label: `changes to ${path}`, summary: `file changes · ${path}` };
  }

  const command = typeof call.input.command === "string" ? call.input.command : undefined;
  if (call.name === "run_command" && command !== undefined) {
    return { key: `command\0${command}`, label: "this exact command", summary: `command · ${command}` };
  }

  return {
    key: `${call.name}\0${stable(call.input)}`,
    label: "this exact call",
    summary: `${call.name} · ${target(call.input)}`,
  };
}

function defaultMode(tool: Tool): PermissionMode {
  return tool.dangerous ? "ask" : "allow";
}

function grantTools(call: ToolCallBlock): readonly string[] {
  return call.name === "write_file" || call.name === "edit_file"
    ? ["edit_file", "write_file"]
    : [call.name];
}

function stable(value: unknown): string {
  if (Array.isArray(value)) return `[${value.map(stable).join(",")}]`;
  if (value !== null && typeof value === "object") {
    return `{${Object.entries(value as Record<string, unknown>)
      .sort(([a], [b]) => a.localeCompare(b))
      .map(([key, item]) => `${JSON.stringify(key)}:${stable(item)}`)
      .join(",")}}`;
  }
  return JSON.stringify(value);
}

function target(input: Record<string, unknown>): string {
  const value = input.path ?? input.command;
  return typeof value === "string" ? value : stable(input);
}
