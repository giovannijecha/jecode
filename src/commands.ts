// Slash-command registry and dispatcher.
//
// One of them does reach the network — a menu of models cannot be built
// without asking the provider what it has — but none of them ever sends a
// message. Provider access and model selection stay in their own commands;
// this file keeps discovery, dispatch, and local session operations.

import type { Session } from "./session.ts";
import { ConversationTree } from "./conversation.ts";
import type { NoticeBlock } from "./tui/blocks.ts";
import type { Picker } from "./tui/picker.ts";
import type { Field } from "./tui/field.ts";
import type { SavedSettings } from "./settings.ts";
import type { SessionPermissions } from "./permissions.ts";
import { modelsCommand } from "./model-command.ts";
import { providersCommand } from "./provider-commands.ts";
import { permissionsCommand } from "./permission-command.ts";
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
  /** Close the active modal after an asynchronous interaction completes. */
  dismiss?(): void;
  /** Take a line from the user. Resolves with nothing if they backed out. */
  type?(field: Field): Promise<string | undefined>;
  status?(text?: string): void;
  signal?: AbortSignal;
  /** Clear conversation-local state, including tool policies and approvals. */
  reset?(): void | Promise<void>;
  permissions?: SessionPermissions;
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
 * model, provider-access, and effort commands remain useful direct paths into
 * the same interactions.
 */
export const COMMANDS: readonly Command[] = [
  { name: "help", blurb: "show keyboard controls" },
  { name: "exit", blurb: "exit and restore the terminal" },
  { name: "new", blurb: "start clean and reset tool permissions" },
  { name: "export", blurb: "save this transcript as Markdown" },
  { name: "permissions", blurb: "manage session tool access" },
  { name: "settings", blurb: "change and save jecode defaults" },
  { name: "effort", blurb: "set the reasoning effort" },
  { name: "models", blurb: "choose from every available provider" },
  { name: "providers", blurb: "manage provider access and connections" },
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
      await host.reset?.();
      session.conversation = ConversationTree.empty();
      session.usage = emptyUsage();
      host.emit({ kind: "notice", text: "new session", tone: "info" });
      return "handled";

    case "export":
      if (host.exportTranscript === undefined) {
        host.emit({ kind: "notice", text: "export needs the interactive screen", tone: "warn" });
      } else {
        const saved = await host.exportTranscript();
        host.emit({ kind: "notice", text: `saved · ${saved}`, tone: "info" });
      }
      return "handled";

    case "permissions":
      await permissionsCommand(session, host);
      return "handled";

    case "settings":
      await settingsCommand(session, host);
      return "handled";

    case "effort":
      await effortCommand(session, host);
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

function chooser(host: Host): Host["choose"] {
  if (host.choose === undefined) {
    host.emit({ kind: "notice", text: "that command needs the screen", tone: "warn" });
  }
  return host.choose;
}
