// The persistent settings hub. Every interaction uses the shared dock picker
// and field contracts; this module owns choices and persistence, not drawing.

import type { Session } from "./session.ts";
import type { Host } from "./commands.ts";
import { modelsCommand, providersCommand } from "./provider-commands.ts";
import { credentialsCommand } from "./credential-commands.ts";
import { EFFORTS, readSettings, settingsLabel, updateSettings } from "./settings.ts";
import type { SavedSettings } from "./settings.ts";
import { providerFailure } from "./provider-errors.ts";
import {
  ollamaConnectionHint,
  ollamaConnectionSetting,
} from "./ollama-settings-command.ts";
import type { Palette } from "./ui/theme.ts";
import { of } from "./tui/editor.ts";
import type { Field } from "./tui/field.ts";
import type { Picker } from "./tui/picker.ts";
import { heading } from "./tui/picker.ts";

type SettingsAction =
  | "provider"
  | "ollamaConnection"
  | "model"
  | "effort"
  | "maxTokens"
  | "maxSteps"
  | "reducedMotion"
  | "credentials";

type SettingsItem = {
  action: SettingsAction;
  option: Picker["options"][number];
};

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
    const values = settingsValues(session);
    const items = settingsItems(values);
    const index = await choose(settingsPicker(values, session.palette, selected));
    if (index === undefined) return;
    const action = items[index]?.action;
    if (action === undefined) return;
    selected = index;

    switch (action) {
      case "provider":
        await providerSetting(session, host);
        break;
      case "ollamaConnection":
        await ollamaConnectionSetting(session, host, (patch) => persist(host, patch));
        break;
      case "model":
        await modelSetting(session, host);
        break;
      case "effort":
        await effortSetting(session, host);
        break;
      case "maxTokens":
        await numberSetting(session, host, "maxTokens", "max output tokens");
        break;
      case "maxSteps":
        await numberSetting(session, host, "maxSteps", "max tool steps");
        break;
      case "reducedMotion":
        await motionSetting(session, host);
        break;
      case "credentials":
        await credentialsCommand(session, host);
        break;
    }
  }
}

export type SettingsValues = {
  provider: string;
  model: string;
  ollamaConnection?: string;
  effort: string;
  maxTokens?: number;
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
    description: "Changes apply now · flags and environment win at launch",
    options: settingsItems(values).map((item) => item.option),
    index,
  };
}

function settingsItems(values: SettingsValues): SettingsItem[] {
  return [
    { action: "provider", option: { label: "provider", hint: values.provider } },
    ...(values.ollamaConnection === undefined
      ? []
      : [{
          action: "ollamaConnection" as const,
          option: { label: "ollama connection", hint: values.ollamaConnection },
        }]),
    { action: "model", option: { label: "model", hint: values.model || "choose a model" } },
    { action: "effort", option: { label: "effort", hint: values.effort } },
    ...(values.maxTokens === undefined
      ? []
      : [{
          action: "maxTokens" as const,
          option: { label: "max output tokens", hint: String(values.maxTokens) },
        }]),
    { action: "maxSteps", option: { label: "max tool steps", hint: String(values.maxSteps) } },
    {
      action: "reducedMotion",
      option: { label: "reduced motion", hint: values.reducedMotion ? "on" : "off" },
    },
    {
      action: "credentials",
      option: { label: "authentication", hint: "manage API keys and accounts" },
    },
  ];
}

function settingsValues(session: Session): SettingsValues {
  return {
    provider: session.provider.id,
    model: session.model,
    ...(session.provider.id === "ollama" ? { ollamaConnection: ollamaConnectionHint() } : {}),
    effort: session.config.effort,
    ...(session.provider.id === "openai-codex" ? {} : { maxTokens: session.config.maxTokens }),
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
    effort: session.config.effort,
  };
  if (!(await providersCommand(session, host, { announce: false, save: false }))) return;

  const current = readSettings();
  const models = { ...current.models };
  if (session.model !== "") models[session.provider.id] = session.model;
  const patch = {
    provider: session.provider.id,
    models,
    ...(session.config.effort === before.effort ? {} : { effort: session.config.effort }),
  };
  if (await persist(host, patch)) return;

  session.provider = before.provider;
  session.model = before.model;
  session.config.providerId = before.providerId;
  session.config.model = before.configModel;
  session.config.effort = before.effort;
}

async function modelSetting(session: Session, host: Host): Promise<void> {
  const before = {
    model: session.model,
    configModel: session.config.model,
    effort: session.config.effort,
  };
  if (!(await modelsCommand(session, host, { announce: false, save: false }))) return;

  const current = readSettings();
  const models = { ...current.models, [session.provider.id]: session.model };
  const patch = {
    models,
    ...(session.config.effort === before.effort ? {} : { effort: session.config.effort }),
  };
  if (await persist(host, patch)) return;

  session.model = before.model;
  session.config.model = before.configModel;
  session.config.effort = before.effort;
}

async function effortSetting(session: Session, host: Host): Promise<string | undefined> {
  const choose = chooser(host);
  if (choose === undefined) return;
  const efforts = await availableEfforts(session, host);
  if (efforts === undefined) return;
  if (efforts.length === 0) {
    host.emit({
      kind: "notice",
      text: `${session.model || session.provider.id} controls its own reasoning depth`,
      tone: "info",
    });
    return;
  }
  const current = session.config.effort;
  const index = await choose({
    title: heading("effort", "saved default", session.palette),
    options: efforts.map((value) => ({ label: value })),
    index: Math.max(0, efforts.findIndex((value) => value === current)),
  });
  const value = index === undefined ? undefined : efforts[index];
  if (value === undefined || !(await persist(host, { effort: value }))) return;
  session.config.effort = value;
  return value;
}

async function availableEfforts(
  session: Session,
  host: Host,
): Promise<readonly string[] | undefined> {
  if (session.provider.efforts === undefined) return EFFORTS;
  host.status?.(`Asking ${session.provider.id}`);
  try {
    return await session.provider.efforts(
      session.model,
      host.signal,
      (status) => host.status?.(status),
    );
  } catch (error) {
    host.emit({
      kind: "notice",
      text: providerFailure(session.provider, error as Error, true),
      tone: "error",
    });
    return undefined;
  } finally {
    host.status?.(undefined);
  }
}

async function motionSetting(session: Session, host: Host): Promise<void> {
  const choose = chooser(host);
  if (choose === undefined) return;
  const values = [false, true];
  const index = await choose({
    title: heading("reduced motion", "saved default", session.palette),
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
    host.emit({ kind: "notice", text: `could not save settings · ${(error as Error).message}`, tone: "error" });
    return false;
  }
}

function chooser(host: Host): Host["choose"] {
  if (host.choose === undefined) {
    host.emit({ kind: "notice", text: "that command needs the screen", tone: "warn" });
  }
  return host.choose;
}
