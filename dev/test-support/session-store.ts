// Shared durable-session fixtures; each caller owns and removes its temporary root.

import { mkdir, mkdtemp } from "node:fs/promises";
import { tmpdir } from "node:os";
import * as path from "node:path";
import type { ConversationTree } from "../../src/conversation.ts";
import type { DurableSessionStore } from "../../src/sessions/store.ts";
import type { Message } from "../../src/types.ts";

export function turn(
  conversation: ConversationTree,
  parentId: number,
  user: string,
  answer: string,
): ConversationTree {
  return conversation.commit({
    parentId,
    createdAt: "2026-09-01T10:00:00.000Z",
    identity: { providerId: "openai-codex", model: "gpt-5.6-terra", effort: "medium" },
    messages: completed(user, answer),
    blocks: [{ kind: "user", text: user }, { kind: "answer", text: answer }],
  }, "completed");
}

export function revise(conversation: ConversationTree, answer: string): ConversationTree {
  const active = conversation.activeNode as NonNullable<typeof conversation.activeNode>;
  return conversation.commit({
    nodeId: active.id,
    parentId: active.parentId,
    createdAt: active.createdAt,
    identity: active.identity,
    messages: completed("first", answer),
    blocks: [{ kind: "user", text: "first" }, { kind: "answer", text: answer }],
  }, "completed");
}

function completed(user: string, answer: string): Message[] {
  return [
    { role: "user", content: [{ kind: "text", text: user }] },
    {
      role: "assistant",
      content: [{ kind: "text", text: answer }],
      raw: { encrypted: answer },
      rawFrom: "openai-codex",
    },
  ];
}

export function stripRaw(message: Message): Message {
  return {
    role: message.role,
    content: message.content,
    ...(message.usage === undefined ? {} : { usage: message.usage }),
  };
}

export async function loadWithLease(
  store: DurableSessionStore,
  id: string,
): Promise<Awaited<ReturnType<DurableSessionStore["load"]>>> {
  const lease = await store.claim(id);
  try {
    return await store.load(id, lease);
  } finally {
    await lease.close();
  }
}

export async function checkpointWithLease(
  store: DurableSessionStore,
  snapshot: Parameters<DurableSessionStore["checkpoint"]>[0],
  conversation: ConversationTree,
): Promise<Awaited<ReturnType<DurableSessionStore["checkpoint"]>>> {
  const lease = await store.claim(snapshot.meta.id);
  try {
    return await store.checkpoint(snapshot, conversation, lease);
  } finally {
    await lease.close();
  }
}

export async function sessionFixture(): Promise<{
  root: string;
  workspace: string;
  sessions: string;
}> {
  const root = await mkdtemp(path.join(tmpdir(), "jecode-sessions-"));
  const workspace = path.join(root, "workspace");
  const sessions = path.join(root, "data", "sessions");
  await mkdir(workspace);
  return { root, workspace, sessions };
}
