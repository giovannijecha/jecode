import { test } from "node:test";
import assert from "node:assert/strict";
import type { Message } from "../src/types.ts";
import { runTurn } from "../src/controller.ts";
import { steeringInbox } from "../src/steering.ts";
import type { Tool } from "../src/tools/index.ts";
import { scripted, echo, options, events, assistantText, texts } from "../dev/test-support/controller.ts";

test("steering queued during a final response continues the same turn", async () => {
  const provider = scripted([assistantText("first answer"), assistantText("revised answer")]);
  const sent: Message[][] = [];
  const send = provider.send.bind(provider);
  provider.send = async (request) => {
    sent.push(structuredClone(request.messages));
    return send(request);
  };
  const inbox = steeringInbox();
  const history: Message[] = [
    { role: "user", content: [{ kind: "text", text: "initial request" }] },
  ];
  const settlements: string[] = [];
  const steered: string[] = [];
  const sink = events();
  sink.onStream = (event) => {
    if (event.kind !== "text") return;
    sink.texts.push(event.text);
    if (event.text === "first answer") inbox.offer("change direction");
  };
  sink.onSteering = (text) => steered.push(text);
  sink.onCheckpoint = async (_checkpoint, settlement) => {
    settlements.push(settlement);
  };

  await runTurn(history, options(provider, { steering: inbox }), sink);

  assert.equal(provider.seen.length, 2);
  assert.deepEqual(texts(sent[1] ?? []), [
    "initial request",
    "first answer",
    "change direction",
  ]);
  assert.deepEqual(texts(history), [
    "initial request",
    "first answer",
    "change direction",
    "revised answer",
  ]);
  assert.deepEqual(steered, ["change direction"]);
  assert.deepEqual(settlements, ["checkpointed", "completed"]);
  assert.equal(inbox.accepting, false);
});

test("an explicit request budget stops a steering continuation without losing guidance", async () => {
  const provider = scripted([assistantText("first answer"), assistantText("not reached")]);
  const inbox = steeringInbox();
  const history: Message[] = [
    { role: "user", content: [{ kind: "text", text: "initial request" }] },
  ];
  const settlements: string[] = [];
  const sink = events();
  sink.onStream = (event) => {
    if (event.kind === "text") inbox.offer("change direction");
  };
  sink.onCheckpoint = async (_checkpoint, settlement) => {
    settlements.push(settlement);
  };

  await assert.rejects(
    runTurn(
      history,
      options(provider, { maxModelRequests: 1, steering: inbox }),
      sink,
    ),
    /stopped after 1 model request/,
  );

  assert.equal(provider.seen.length, 1);
  assert.deepEqual(texts(history), [
    "initial request",
    "first answer",
    "change direction",
  ]);
  assert.deepEqual(settlements, ["checkpointed"]);
  assert.equal(inbox.pending, 0);
});

test("steering waits for the complete issued tool batch", async () => {
  const inbox = steeringInbox();
  const steeringTool: Tool = {
    ...echo,
    concurrency: "exclusive",
    async run(args) {
      inbox.offer("do not touch the README");
      return { output: String(args.text) };
    },
  };
  const provider = scripted([
    {
      role: "assistant",
      content: [{ kind: "tool_call", id: "a", name: "echo", input: { text: "done" } }],
    },
    assistantText("acknowledged"),
  ]);

  await runTurn([], options(provider, { tools: [steeringTool], steering: inbox }), events());

  const followup = provider.seen[1]?.messages ?? [];
  assert.equal(followup[0]?.role, "assistant");
  assert.equal(followup[1]?.content[0]?.kind, "tool_result");
  assert.equal(followup[2]?.role, "user");
  assert.equal(texts([followup[2] as Message])[0], "do not touch the README");
});
