// One catalogue supplies discovery, navigation, command previews, and tests.

import type { LabState, Scenario } from "./model.ts";
import * as conversation from "./scenarios/conversation.ts";
import * as tools from "./scenarios/tools.ts";
import * as approvals from "./scenarios/approvals.ts";
import * as input from "./scenarios/input.ts";
import * as sessions from "./scenarios/sessions.ts";
import * as configuration from "./scenarios/configuration.ts";
import { MENU_MOMENTS, menuScene } from "./scenarios/menu-workflow.ts";
import { workflowScene, WORKFLOW_DURATION_MS, WORKFLOW_MOMENTS } from "./scenarios/workflow.ts";

export const SCENARIOS = [
  { id: "golden", title: "Complete conversation frame", group: "Conversation", create: conversation.goldenScene },
  { id: "conversation", title: "Settled conversation", group: "Conversation", create: conversation.conversationScene },
  { id: "tools-live", title: "Running file edit", group: "Tools", create: tools.toolsLiveScene, animated: true },
  { id: "tools-trace", title: "Failure and investigation", group: "Tools", create: tools.toolsTraceScene, animated: true },
  { id: "tools-output", title: "Completed command output", group: "Tools", create: tools.toolsOutputScene },
  { id: "tools-stream", title: "Streaming command output", group: "Tools", create: tools.toolsStreamScene, animated: true },
  { id: "tools-diff", title: "File changes", group: "Tools", create: tools.toolsDiffScene },
  { id: "approve-edit", title: "File approval", group: "Approvals", create: approvals.approveEditScene },
  { id: "approve-command", title: "Command approval", group: "Approvals", create: approvals.approveCommandScene },
  { id: "approve-denied", title: "Denied file edit", group: "Approvals", create: approvals.approveDeniedScene },
  { id: "menu-commands", title: "Command completion", group: "Input", create: input.commandsScene },
  { id: "menu-search", title: "Searchable model catalogue", group: "Input", create: input.searchScene, command: "/models" },
  { id: "menu-resume", title: "Saved conversations", group: "Sessions", create: sessions.resumeScene },
  { id: "menu-timeline", title: "Conversation branches", group: "Sessions", create: sessions.timelineScene, command: "/timeline" },
  { id: "menu-settings", title: "Settings", group: "Configuration", create: configuration.settingsScene, command: "/settings" },
  { id: "menu-permissions", title: "Tool permissions", group: "Configuration", create: configuration.permissionsScene, command: "/permissions" },
  { id: "help", title: "Keyboard reference", group: "Input", create: input.helpScene, command: "/help" },
  { id: "field", title: "Masked credential field", group: "Input", create: input.fieldScene },
  { id: "markdown", title: "Markdown and code", group: "Conversation", create: conversation.markdownScene },
  { id: "reasoning", title: "Live reasoning", group: "Conversation", create: conversation.reasoningScene },
  { id: "feedback", title: "Blocked send with retained prompt", group: "Conversation", create: conversation.feedbackScene },
  { id: "scroll", title: "Long transcript and scrolling", group: "Conversation", create: conversation.scrollScene },
  { id: "steering", title: "Guidance and interruption", group: "Conversation", create: conversation.steeringScene, animated: true },
  { id: "reasoning-stream", title: "Reasoning arrival and settlement", group: "Conversation", create: conversation.reasoningStreamScene, animated: true, durationMs: 8_000 },
  { id: "tools-lifecycle", title: "Waiting, running, and completed", group: "Tools", create: tools.lifecycleScene, animated: true, durationMs: 4_000 },
  { id: "tools-workflow", title: "Read, diagnose, edit, write, and verify", group: "Tools", create: workflowScene, animated: true, durationMs: WORKFLOW_DURATION_MS, moments: WORKFLOW_MOMENTS },
  { id: "menu-providers", title: "Provider access groups", group: "Configuration", create: configuration.providersScene, command: "/providers", select: (index: number) => index === 0 ? "providers-account" : "providers-api" },
  { id: "providers-account", title: "OpenAI Account access", group: "Configuration", create: configuration.providerAccountScene },
  { id: "providers-api", title: "API access", group: "Configuration", create: configuration.providerApiScene },
  { id: "menu-workflow", title: "Commands, selectors, and approvals", group: "Input", create: menuScene,
    moments: MENU_MOMENTS, routes: { "/models": 1_000, "/settings": 2_000, "/permissions": 3_000, "/help": 7_000 } },
] as const satisfies readonly Scenario[];

export type Scene = (typeof SCENARIOS)[number]["id"];
export const SCENES: readonly Scene[] = SCENARIOS.map((scenario) => scenario.id);

export function scenarioFor(id: string): Scenario {
  const scenario = SCENARIOS.find((candidate) => candidate.id === id);
  if (scenario === undefined) throw new Error(`unknown lab scene: ${id}`);
  return scenario;
}

export function sceneView(state: LabState) {
  const view = scenarioFor(state.scene).create(state);
  return { ...view, scroll: state.scroll ?? view.scroll, reducedMotion: state.reducedMotion === true };
}
