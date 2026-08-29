// The persistent settings hub. Every interaction uses the shared dock picker
// and field contracts; this module owns choices and persistence, not drawing.

import type { Session } from "./session.ts";
import type { Host } from "./commands.ts";
import { modelsCommand, providersCommand } from "./provider-commands.ts";
import { credentialsCommand } from "./credential-commands.ts";
import { EFFORTS, readSettings, settingsLabel, updateSettings } from "./settings.ts";
import type { SavedSettings } from "./settings.ts";
import type { Palette } from "./ui/theme.ts";
import { of } from "./tui/editor.ts";
import type { Field } from "./tui/field.ts";
import type { Picker } from "./tui/picker.ts";
import { heading } from "./tui/picker.ts";

const CLOSE = 7;

/** A focused path to the same saved reasoning default exposed by /settings. */
export async function effortCommand(session: Session, host: Host): Promise<void> {
  const value = await effortSetting(session, host);
  if (value === undefined) return;
  host.emit({ kind: "notice", text: `effort · ${value}`, tone: "info" });
}

export async function settingsCommand(session: Session, host: Host): Promise<void> {
  const choose = chooser(host);
  if (choose === undefined) return;

  let selected = 0;
  while (true) {
    const index = await choose(settingsPicker(settingsValues(session), session.palette, selected));
    if (index === undefined || index === CLOSE) return;
    selected = index;

    switch (index) {
      case 0:
        await providerSetting(session, host);
        break;
      case 1:
        await modelSetting(session, host);
        break;
      case 2:
        await effortSetting(session, host);
        break;
      case 3:
        await numberSetting(session, host, "maxTokens", "max output tokens");
        break;
      case 4:
        await numberSetting(session, host, "maxSteps", "max tool steps");
        break;
      case 5:
        await motionSetting(session, host);
        break;
      case 6:
        await credentialsCommand(session, host);
        break;
    }
  }
}

export type SettingsValues = {
  provider: string;
  model: string;
  effort: string;
  maxTokens: number;
  maxSteps: number;
  reducedMotion: boolean;
};

export function settingsPicker(
  values: SettingsValues,
  pal: Palette,
  index = 0,
  store = settingsLabel(),
): Picker {
  return {
    title: heading("settings", store, pal),
    right: "↑↓ enter · esc close",
    footer: "Changes apply now · flags and environment win at launch",
    options: [
      { label: "provider", hint: values.provider },
      { label: "model", hint: values.model || "choose a model" },
      { label: "effort", hint: values.effort },
      { label: "max output tokens", hint: String(values.maxTokens) },
      { label: "max tool steps", hint: String(values.maxSteps) },
      { label: "reduced motion", hint: values.reducedMotion ? "on" : "off" },
      { label: "credentials", hint: "manage API keys" },
      { label: "close" },
    ],
    index,
  };
}

function settingsValues(session: Session): SettingsValues {
  return {
    provider: session.provider.id,
    model: session.model,
    effort: session.config.effort,
    maxTokens: session.config.maxTokens,
    maxSteps: session.config.maxSteps,
    reducedMotion: session.config.reducedMotion,
  };
}

async function providerSetting(session: Session, host: Host): Promise<void> {
  const before = {
    provider: session.provider,
    model: session.model,
    providerId: session.config.providerId,
    configModel: session.config.model,
  };
  if (!(await providersCommand(session, host, { announce: false, save: false }))) return;

  const current = readSettings();
  const models = { ...current.models };
  if (session.model !== "") models[session.provider.id] = session.model;
  if (await persist(host, { provider: session.provider.id, models })) return;

  session.provider = before.provider;
  session.model = before.model;
  session.config.providerId = before.providerId;
  session.config.model = before.configModel;
}

async function modelSetting(session: Session, host: Host): Promise<void> {
  const before = { model: session.model, configModel: session.config.model };
  if (!(await modelsCommand(session, host, { announce: false, save: false }))) return;

  const current = readSettings();
  const models = { ...current.models, [session.provider.id]: session.model };
  if (await persist(host, { models })) return;

  session.model = before.model;
  session.config.model = before.configModel;
}

async function effortSetting(session: Session, host: Host): Promise<string | undefined> {
  const choose = chooser(host);
  if (choose === undefined) return;
  const current = session.config.effort;
  const index = await choose({
    title: heading("effort", "saved default", session.palette),
    right: "↑↓ enter · esc back",
    options: EFFORTS.map((value) => ({ label: value })),
    index: Math.max(0, EFFORTS.findIndex((value) => value === current)),
  });
  const value = index === undefined ? undefined : EFFORTS[index];
  if (value === undefined || !(await persist(host, { effort: value }))) return;
  session.config.effort = value;
  return value;
}

async function motionSetting(session: Session, host: Host): Promise<void> {
  const choose = chooser(host);
  if (choose === undefined) return;
  const values = [false, true];
  const index = await choose({
    title: heading("reduced motion", "saved default", session.palette),
    right: "↑↓ enter · esc back",
    options: values.map((value) => ({ label: value ? "on" : "off" })),
    index: session.config.reducedMotion ? 1 : 0,
  });
  const value = index === undefined ? undefined : values[index];
  if (value === undefined || !(await persist(host, { reducedMotion: value }))) return;
  session.config.reducedMotion = value;
  host.refreshSettings?.();
}

async function numberSetting(
  session: Session,
  host: Host,
  name: "maxTokens" | "maxSteps",
  label: string,
): Promise<void> {
  if (host.type === undefined) return;
  const field: Field = {
    title: heading(label, "positive integer", session.palette),
    right: "enter save · esc back",
    editor: of(String(session.config[name])),
    secret: false,
    note: "Applies to the next model turn.",
  };
  const text = await host.type(field);
  if (text === undefined) return;
  const value = Number(text);
  if (!Number.isSafeInteger(value) || value <= 0) {
    host.emit({ kind: "notice", text: `${label} must be a positive integer`, tone: "error" });
    return;
  }
  if (!(await persist(host, { [name]: value }))) return;
  session.config[name] = value;
}

async function persist(host: Host, patch: Partial<SavedSettings>): Promise<boolean> {
  try {
    await updateSettings(patch);
    return true;
  } catch (error) {
    host.emit({ kind: "notice", text: `could not save settings: ${(error as Error).message}`, tone: "error" });
    return false;
  }
}

function chooser(host: Host): Host["choose"] {
  if (host.choose === undefined) {
    host.emit({ kind: "notice", text: "that command needs the screen", tone: "warn" });
  }
  return host.choose;
}
