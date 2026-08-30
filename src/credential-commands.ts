// Credential command flows shared by setup and provider selection.

import type { Session } from "./session.ts";
import type { Host } from "./commands.ts";
import type { Option } from "./tui/picker.ts";
import { heading } from "./tui/picker.ts";
import type { Field } from "./tui/field.ts";
import { EMPTY } from "./tui/editor.ts";
import { PROVIDERS } from "./providers/index.ts";
import {
  credentialSource,
  forgetSaved,
  hasSaved,
  hold,
  keep,
  storeLabel,
} from "./credentials.ts";
import type { Palette } from "./ui/theme.ts";

/** Ask for a key, then ask separately whether it may be written to disk. */
export async function askForKey(name: string, host: Host, pal: Palette): Promise<boolean> {
  if (host.type === undefined || host.choose === undefined) return false;

  const field: Field = {
    title: [
      { text: "paste key  ", fg: pal.ink.attention, bold: true },
      { text: name, fg: pal.ink.bright, bold: true },
    ],
    right: "enter ok · esc skip",
    editor: EMPTY,
    secret: true,
    note: "read from the environment first — an exported value still wins",
  };

  const value = await host.type(field);
  if (value === undefined) return false;

  const index = await host.choose({
    title: heading("remember it?", name, pal),
    options: [
      { label: "just this session", hint: "nothing is written", key: "s" },
      { label: `save it to ${storeLabel()}`, hint: "owner-only file", key: "w" },
      { label: "discard it", key: "d" },
    ],
    index: 0,
  });

  if (index === 0) {
    hold(name, value);
    host.emit({ kind: "notice", text: "credential available for this session", tone: "info" });
    return true;
  }

  if (index === 1) {
    try {
      await keep(name, value);
      host.emit({ kind: "notice", text: `credential saved · ${storeLabel()}`, tone: "info" });
      return true;
    } catch (error) {
      host.emit({ kind: "notice", text: `could not save credential · ${(error as Error).message}`, tone: "error" });
      return false;
    }
  }

  host.emit({ kind: "notice", text: "credential discarded", tone: "warn" });
  return false;
}

export async function credentialsCommand(session: Session, host: Host): Promise<void> {
  const choose = chooser(host);
  if (choose === undefined) return;

  const index = await choose({
    title: heading("credential", "values are never shown", session.palette),
    options: PROVIDERS.map((provider) => ({
      label: provider.keyVar,
      hint: credentialSource(provider.keyVar) ?? "missing",
    })),
    index: Math.max(0, PROVIDERS.findIndex((provider) => provider.id === session.provider.id)),
  });
  if (index === undefined) return;

  const provider = PROVIDERS[index];
  if (provider === undefined) return;
  const name = provider.keyVar;
  const source = credentialSource(name);

  if (source === "environment") {
    host.emit({
      kind: "notice",
      text: `${name} comes from the environment · update it outside jecode and restart`,
      tone: "info",
    });
    if (hasSaved(name)) await offerForget(name, session, host, "a saved copy is currently shadowed");
    return;
  }

  const actions: Option[] = [
    { label: source === undefined ? "add credential" : "replace credential", key: "r" },
    ...(hasSaved(name) ? [{ label: "forget saved copy", hint: storeLabel(), key: "f" }] : []),
    { label: "close", key: "c" },
  ];
  const action = await choose({
    title: heading(name, source ?? "missing", session.palette),
    options: actions,
    index: 0,
  });
  if (action === undefined || action === actions.length - 1) return;
  if (actions[action]?.key === "f") {
    await forget(name, host);
    return;
  }
  await askForKey(name, host, session.palette);
}

async function offerForget(name: string, session: Session, host: Host, hint: string): Promise<void> {
  if (host.choose === undefined) return;
  const index = await host.choose({
    title: heading(name, hint, session.palette),
    options: [
      { label: "keep saved copy", key: "k" },
      { label: "forget saved copy", hint: storeLabel(), key: "f" },
    ],
    index: 0,
  });
  if (index === 1) await forget(name, host);
}

async function forget(name: string, host: Host): Promise<void> {
  try {
    const removed = await forgetSaved(name);
    host.emit({
      kind: "notice",
      text: removed ? "saved credential removed" : "no saved credential to remove",
      tone: removed ? "info" : "warn",
    });
  } catch (error) {
    host.emit({ kind: "notice", text: `could not forget: ${(error as Error).message}`, tone: "error" });
  }
}

function chooser(host: Host): Host["choose"] {
  if (host.choose === undefined) {
    host.emit({ kind: "notice", text: "that command needs the screen", tone: "warn" });
  }
  return host.choose;
}
