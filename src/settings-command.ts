// The persistent settings hub. Provider access and model selection reuse the
// same command flows exposed directly through /providers and /models.

import { saveCommandSettings } from "./command-settings.ts";
import type { Host } from "./commands.ts";
import { MAX_COMPACTION_PERCENT, MIN_COMPACTION_PERCENT } from "./context/policy.ts";
import { modelsCommand } from "./model-command.ts";
import { providerFailure } from "./provider-errors.ts";
import { providerLabel } from "./provider-label.ts";
import { providersCommand } from "./provider-commands.ts";
import type { Session } from "./session.ts";
import { EFFORTS } from "./settings.ts";
import { of } from "./tui/editor.ts";
import type { Field } from "./tui/field.ts";
import type { Picker } from "./tui/picker.ts";
import { heading } from "./tui/picker.ts";

type SettingsAction =
  | "model"
  | "effort"
  | "maxTokens"
  | "compactionPercent"
  | "reducedMotion"
  | "providers";

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
    const index = await choose(settingsPicker(values, selected));
    if (index === undefined) return;
    const action = items[index]?.action;
    if (action === undefined) return;
    selected = index;

    switch (action) {
      case "model":
        await modelsCommand(session, host, { announce: false });
        break;
      case "effort":
        await effortSetting(session, host);
        break;
      case "maxTokens":
        await numberSetting(session, host, "maxTokens", "max output tokens");
        break;
      case "compactionPercent":
        await compactionSetting(session, host);
        break;
      case "reducedMotion":
        await motionSetting(session, host);
        break;
      case "providers":
        await providersCommand(session, host);
        break;
    }
  }
}

export type SettingsValues = {
  provider: string;
  model: string;
  effort: string;
  maxTokens?: number;
  compactionPercent: number;
  reducedMotion: boolean;
};

export function settingsPicker(
  values: SettingsValues,
  index = 0,
): Picker {
  const items = settingsItems(values);
  return {
    title: [],
    options: items.map((item) => item.option),
    visible: items.length,
    index,
  };
}

function settingsItems(values: SettingsValues): SettingsItem[] {
  return [
    {
      action: "model",
      option: {
        label: "model",
        value: `${providerLabel(values.provider)} · ${values.model || "choose a model"}`,
      },
    },
    { action: "effort", option: { label: "effort", value: values.effort } },
    ...(values.maxTokens === undefined
      ? []
      : [{
          action: "maxTokens" as const,
          option: { label: "max output tokens", value: String(values.maxTokens) },
        }]),
    {
      action: "compactionPercent",
      option: { label: "context compaction", value: `${values.compactionPercent}%` },
    },
    {
      action: "reducedMotion",
      option: { label: "reduced motion", value: values.reducedMotion ? "on" : "off" },
    },
    {
      action: "providers",
      option: { label: "providers", hint: "manage connections" },
    },
  ];
}

function settingsValues(session: Session): SettingsValues {
  return {
    provider: session.provider.id,
    model: session.model,
    effort: session.config.effort,
    ...(session.provider.id === "openai-codex" ? {} : { maxTokens: session.config.maxTokens }),
    compactionPercent: session.config.compactionPercent,
    reducedMotion: session.config.reducedMotion,
  };
}

async function effortSetting(session: Session, host: Host): Promise<string | undefined> {
  const choose = chooser(host);
  if (choose === undefined) return;
  const efforts = await availableEfforts(session, host);
  throwIfAborted(host.signal);
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
    title: [],
    options: efforts.map((value) => ({ label: value })),
    index: Math.max(0, efforts.findIndex((value) => value === current)),
  });
  const value = index === undefined ? undefined : efforts[index];
  if (value === undefined || !(await saveCommandSettings(host, { effort: value }))) return;
  session.config.effort = value;
  return value;
}

async function availableEfforts(
  session: Session,
  host: Host,
): Promise<readonly string[] | undefined> {
  if (session.provider.efforts === undefined) return EFFORTS;
  host.status?.(`Asking ${providerLabel(session.provider.id)}`);
  try {
    throwIfAborted(host.signal);
    const efforts = await session.provider.efforts(
      session.model,
      host.signal,
      (status) => host.status?.(status),
    );
    throwIfAborted(host.signal);
    return efforts;
  } catch (error) {
    throwIfAborted(host.signal);
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
    title: [],
    options: values.map((value) => ({ label: value ? "on" : "off" })),
    index: session.config.reducedMotion ? 1 : 0,
  });
  const value = index === undefined ? undefined : values[index];
  if (value === undefined || !(await saveCommandSettings(host, { reducedMotion: value }))) return;
  session.config.reducedMotion = value;
  host.refreshSettings?.();
}

async function numberSetting(
  session: Session,
  host: Host,
  name: "maxTokens",
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
  if (!(await saveCommandSettings(host, { [name]: value }))) return;
  session.config[name] = value;
}

async function compactionSetting(session: Session, host: Host): Promise<void> {
  if (host.type === undefined) return;
  const label = "context compaction";
  const field: Field = {
    title: heading(
      label,
      `${MIN_COMPACTION_PERCENT}-${MAX_COMPACTION_PERCENT} percent`,
      session.palette,
    ),
    right: "enter save · esc back",
    editor: of(String(session.config.compactionPercent)),
    secret: false,
    note: "Compacts when model context reaches this percentage.",
  };
  const text = await host.type(field);
  if (text === undefined) return;
  const value = Number(text);
  if (
    !Number.isSafeInteger(value) ||
    value < MIN_COMPACTION_PERCENT ||
    value > MAX_COMPACTION_PERCENT
  ) {
    host.emit({
      kind: "notice",
      text: `${label} must be from ${MIN_COMPACTION_PERCENT} to ${MAX_COMPACTION_PERCENT}`,
      tone: "error",
    });
    return;
  }
  if (!(await saveCommandSettings(host, { compactionPercent: value }))) return;
  session.config.compactionPercent = value;
}

function chooser(host: Host): Host["choose"] {
  if (host.choose === undefined) {
    host.emit({ kind: "notice", text: "that command needs the screen", tone: "warn" });
  }
  return host.choose;
}

function throwIfAborted(signal: AbortSignal | undefined): void {
  if (signal?.aborted !== true) return;
  throw signal.reason instanceof Error ? signal.reason : new Error("interrupted");
}
