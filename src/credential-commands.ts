// Masked API-key interaction shared by provider connection flows.

import type { Host } from "./commands.ts";
import {
  credentialSource,
  forgetSaved,
  forgetSession,
  hasSaved,
  hold,
  keep,
  storeLabel,
} from "./credentials.ts";
import type { Session } from "./session.ts";
import { EMPTY } from "./tui/editor.ts";
import type { Field } from "./tui/field.ts";
import { heading } from "./tui/picker.ts";
import type { Option } from "./tui/picker.ts";
import type { Palette } from "./ui/theme.ts";

type CredentialRoute = {
  label: string;
  active: boolean;
};

/** Ask for a key, then ask separately whether it may be written to disk. */
export async function askForKey(
  name: string,
  host: Host,
  pal: Palette,
  route?: CredentialRoute,
): Promise<boolean> {
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
    host.emit({
      kind: "notice",
      text: readyMessage(route, "ready for this session"),
      tone: "info",
    });
    return true;
  }

  if (index === 1) {
    try {
      await keep(name, value);
      host.emit({ kind: "notice", text: readyMessage(route, "saved"), tone: "info" });
      return true;
    } catch (error) {
      host.emit({
        kind: "notice",
        text: `could not save API key · ${(error as Error).message}`,
        tone: "error",
      });
      return false;
    }
  }

  return false;
}

/** Manage one provider key without ever placing its value on screen. */
export async function apiKeyCommand(
  name: string,
  label: string,
  session: Session,
  host: Host,
  providerId?: string,
): Promise<void> {
  const choose = chooser(host);
  if (choose === undefined) return;

  const source = credentialSource(name);
  if (source === "environment") {
    if (!hasSaved(name)) {
      host.emit({
        kind: "notice",
        text: `${label} API key comes from the environment · restart after changing it`,
        tone: "info",
      });
      return;
    }
    const action = await choose({
      title: heading(`${label} API key`, `${name} · environment`, session.palette),
      description: "The environment value is read-only. A saved copy is currently shadowed.",
      options: [{ label: "forget saved copy", hint: storeLabel(), key: "f" }],
      index: 0,
    });
    if (action === 0) await forgetSavedKey(name, host);
    return;
  }

  const actions: Option[] = [
    {
      label: source === undefined ? "add API key" : "replace API key",
      hint: source === undefined ? "session or owner-only file" : `currently ${source}`,
      key: "r",
    },
    ...(source === "session"
      ? [{ label: "clear session key", hint: "this process only", key: "c" }]
      : []),
    ...(hasSaved(name)
      ? [{ label: "forget saved copy", hint: storeLabel(), key: "f" }]
      : []),
  ];
  const index = await choose({
    title: heading(`${label} API key`, `${name} · ${source ?? "missing"}`, session.palette),
    options: actions,
    index: 0,
  });
  const action = index === undefined ? undefined : actions[index]?.key;
  if (action === "r") {
    await askForKey(name, host, session.palette, {
      label,
      active: providerId !== undefined && providerId === session.provider.id,
    });
  }
  else if (action === "c") clearSessionKey(name, host);
  else if (action === "f") await forgetSavedKey(name, host);
}

function readyMessage(route: CredentialRoute | undefined, state: string): string {
  if (route === undefined) return `API key ${state}`;
  const next = route.active
    ? "current provider route"
    : `choose ${route.label} in /models to use it`;
  return `${route.label} key ${state} · ${next}`;
}

function clearSessionKey(name: string, host: Host): void {
  const removed = forgetSession(name);
  const fallback = credentialSource(name);
  host.emit({
    kind: "notice",
    text: removed
      ? fallback === undefined
        ? "session API key removed"
        : `session API key removed · ${fallback} copy now active`
      : "no session API key",
    tone: removed ? "info" : "warn",
  });
}

async function forgetSavedKey(name: string, host: Host): Promise<void> {
  try {
    const removed = await forgetSaved(name);
    host.emit({
      kind: "notice",
      text: removed ? "saved API key removed" : "no saved API key",
      tone: removed ? "info" : "warn",
    });
  } catch (error) {
    host.emit({
      kind: "notice",
      text: `could not remove API key · ${(error as Error).message}`,
      tone: "error",
    });
  }
}

function chooser(host: Host): Host["choose"] {
  if (host.choose === undefined) {
    host.emit({ kind: "notice", text: "that command needs the screen", tone: "warn" });
  }
  return host.choose;
}
