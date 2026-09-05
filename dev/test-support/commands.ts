import type { Message, Provider, SendRequest } from "../../src/types.ts";
import { ConversationTree } from "../../src/conversation.ts";
import type { Session } from "../../src/session.ts";
import type { NoticeBlock } from "../../src/tui/blocks.ts";
import type { Picker } from "../../src/tui/picker.ts";
import type { Field } from "../../src/tui/field.ts";
import type { Host } from "../../src/commands.ts";
import { STEEL } from "../../src/ui/theme.ts";
import { emptyUsage } from "../../src/usage.ts";

export function provider(id: string, models: string[], why?: string): Provider {
  return {
    id,
    defaultModel: models[0] ?? "",
    auth: { kind: "api-key", keyVar: `${id.toUpperCase()}_API_KEY` },
    blocked: () => why,
    models: () => Promise.resolve(models),
    send: (_req: SendRequest): Promise<Message> => Promise.reject(new Error("not called")),
  };
}

export function session(from: Provider): Session {
  return {
    config: {
      providerId: from.id,
      model: from.defaultModel,
      reducedMotion: false,
      effort: "high",
      maxTokens: 4096,
      compactionPercent: 85,
      root: process.cwd(),
      autoApprove: false,
      ephemeral: false,
    },
    provider: from,
    model: from.defaultModel,
    palette: STEEL,
    tools: [],
    system: "",
    conversation: ConversationTree.empty(),
    usage: emptyUsage(),
  };
}

export type Screen = Host & {
  blocks: NoticeBlock[];
  pickers: Picker[];
  fields: Field[];
  helps: number;
  /** What the next field hands back. Unset means the user backed out. */
  typed?: string;
};

/** A host that answers the menus in the order given, and remembers what it saw. */
export function host(...answers: (number | undefined)[]): Screen {
  const screen: Screen = {
    blocks: [],
    pickers: [],
    fields: [],
    helps: 0,
    emit: (block) => {
      screen.blocks.push(block);
    },
    choose: (picker) => {
      screen.pickers.push(picker);
      return Promise.resolve(answers.shift());
    },
    type: (field) => {
      screen.fields.push(field);
      return Promise.resolve(screen.typed);
    },
    showHelp: () => {
      screen.helps++;
      return Promise.resolve();
    },
  };
  return screen;
}

export function texts(blocks: NoticeBlock[]): string[] {
  return blocks.map((block) => block.text);
}
