// Real menu interactions at named fixture times, using production factories.

import type { LabState } from "../model.ts";
import type { View } from "../../../src/tui/view.ts";
import { TICK_MS } from "../model.ts";
import * as input from "./input.ts";
import * as configuration from "./configuration.ts";
import * as approvals from "./approvals.ts";

const samples = [
  { title: "Commands", create: input.commandsScene },
  { title: "Models", create: input.searchScene },
  { title: "Settings", create: configuration.settingsScene },
  { title: "Permissions", create: configuration.permissionsScene },
  { title: "Edit approval", create: approvals.approveEditScene },
  { title: "Command approval", create: approvals.approveCommandScene },
  { title: "Masked field", create: input.fieldScene },
  { title: "Help", create: input.helpScene },
] as const;

export const MENU_MOMENTS = samples.map(({ title }, index) => ({ title, time: index * 1_000 }));

export function menuScene(state: LabState): View {
  const index = Math.min(samples.length - 1, Math.floor(state.tick * TICK_MS / 1_000));
  const sample = samples[Math.max(0, index)]!;
  const view = sample.create(state);
  if (sample.title !== "Models" || view.modal?.kind !== "pick") return view;
  return { ...view, modal: { kind: "pick", picker: { ...view.modal.picker, query: "" } } };
}
