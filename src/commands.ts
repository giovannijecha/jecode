// Slash-command registry and dispatcher.
//
// One of them does reach the network — a menu of models cannot be built
// without asking the provider what it has — but none of them ever sends a
// message. Provider and credential interaction lives in provider-commands.ts;
// this file keeps command discovery, dispatch, and local session operations.

import type { Session } from "./session.ts";
import type { NoticeBlock } from "./tui/blocks.ts";
import type { Picker } from "./tui/picker.ts";
import { heading } from "./tui/picker.ts";
import type { Field } from "./tui/field.ts";
import type { SavedSettings } from "./settings.ts";
import { modelsCommand, providersCommand, setupCommand } from "./provider-commands.ts";
import { credentialsCommand } from "./credential-commands.ts";
import { effortCommand, settingsCommand } from "./settings-command.ts";
import { emptyUsage } from "./usage.ts";

export type CommandOutcome = "handled" | "exit";
export type Emit = (notice: NoticeBlock) => void;

/**
 * What a command has to work with.
 *
 * `choose` and `status` are optional because the batch path has no screen to
 * put a menu on. A command that needs one says so and stops, which is better
 * than every caller having to invent an answer to a question it cannot ask.
 */
export type Host = {
  emit: Emit;
  /** Open the non-persistent keyboard reference. Resolves when it closes. */
  showHelp?(): Promise<void>;
  choose?(picker: Picker): Promise<number | undefined>;
  /** Take a line from the user. Resolves with nothing if they backed out. */
  type?(field: Field): Promise<string | undefined>;
  status?(text?: string): void;
  signal?: AbortSignal;
  /** Clear transcript-local state such as remembered approvals. */
  reset?(): void;
  permissions?(): readonly { key: string; label: string }[];
  revokePermission?(key?: string): void;
  exportTranscript?(): Promise<string>;
  /** Persist non-secret defaults when the host owns an interactive settings store. */
  saveSettings?(patch: Partial<SavedSettings>): Promise<void>;
  /** Repaint after a live display setting changes. */
  refreshSettings?(): void;
};

export type Command = { name: string; blurb: string };

/**
 * The commands, declared once.
 *
 * Deliberately short. `/settings` owns persistent defaults; the narrower
 * provider, model, effort, credential, and setup commands remain useful direct
 * paths into the same interactions.
 */
export const COMMANDS: readonly Command[] = [
  { name: "help", blurb: "show keyboard controls" },
  { name: "exit", blurb: "exit and restore the terminal" },
  { name: "new", blurb: "start a clean in-memory session" },
  { name: "export", blurb: "save this transcript as Markdown" },
  { name: "permissions", blurb: "review or revoke remembered approvals" },
  { name: "settings", blurb: "change and save jecode defaults" },
  { name: "effort", blurb: "set the reasoning effort" },
  { name: "credentials", blurb: "inspect, replace, or forget API keys" },
  { name: "setup", blurb: "make the current provider ready" },
  { name: "models", blurb: "pick a model, from what the provider offers" },
  { name: "providers", blurb: "pick a provider" },
];

export async function handleCommand(
  line: string,
  session: Session,
  host: Host,
): Promise<CommandOutcome> {
  const [name] = line.slice(1).trim().split(/\s+/);

  switch (name) {
    case "help":
      if (host.showHelp === undefined) {
        host.emit({
          kind: "notice",
          text: "interactive help needs the TUI · run jecode --help for startup options",
          tone: "info",
        });
      } else {
        await host.showHelp();
      }
      return "handled";

    case "exit":
      return "exit";

    case "new":
      session.history.length = 0;
      session.usage = emptyUsage();
      host.reset?.();
      host.emit({ kind: "notice", text: "new session · history, usage, and approvals cleared", tone: "info" });
      return "handled";

    case "export":
      if (host.exportTranscript === undefined) {
        host.emit({ kind: "notice", text: "export needs the interactive screen", tone: "warn" });
      } else {
        const saved = await host.exportTranscript();
        host.emit({ kind: "notice", text: `transcript saved to ${saved}`, tone: "info" });
      }
      return "handled";

    case "permissions":
      await permissions(session, host);
      return "handled";

    case "settings":
      await settingsCommand(session, host);
      return "handled";

    case "effort":
      await effortCommand(session, host);
      return "handled";

    case "credentials":
      await credentialsCommand(session, host);
      return "handled";

    case "setup":
      await setupCommand(session, host);
      return "handled";

    case "models":
      await modelsCommand(session, host);
      return "handled";

    case "providers":
      await providersCommand(session, host);
      return "handled";

    default:
      host.emit({
        kind: "notice",
        text: `unknown command /${name ?? ""} · type / to browse commands`,
        tone: "warn",
      });
      return "handled";
  }
}

async function permissions(session: Session, host: Host): Promise<void> {
  const choose = chooser(host);
  if (choose === undefined) return;
  if (session.config.autoApprove) {
    host.emit({
      kind: "notice",
      text: "--auto-approve is active · every dangerous call is allowed for this process",
      tone: "warn",
    });
  }

  const entries = host.permissions?.() ?? [];
  if (entries.length === 0) {
    host.emit({ kind: "notice", text: "no remembered session permissions", tone: "info" });
    return;
  }

  const index = await choose({
    title: heading("revoke permission", "applies only to this session", session.palette),
    options: [
      ...entries.map((entry) => ({ label: entry.label, hint: "revoke" })),
      { label: "all remembered permissions", hint: "revoke all" },
    ],
    index: 0,
  });
  if (index === undefined) return;
  if (index === entries.length) {
    host.revokePermission?.();
    host.emit({ kind: "notice", text: "all remembered permissions revoked", tone: "info" });
    return;
  }

  const entry = entries[index];
  if (entry === undefined) return;
  host.revokePermission?.(entry.key);
  host.emit({ kind: "notice", text: `revoked · ${entry.label}`, tone: "info" });
}

function chooser(host: Host): Host["choose"] {
  if (host.choose === undefined) {
    host.emit({ kind: "notice", text: "that command needs the screen", tone: "warn" });
  }
  return host.choose;
}
