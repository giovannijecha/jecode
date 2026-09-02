// Canonical settled conversation state.
//
// A node owns one complete user-turn delta. The selected root-to-node path is
// the durable source for the transcript and full history; context anchors can
// project an older prefix before it is sent to a provider. Provider traffic
// and live screen blocks remain prospective until a consistent checkpoint.

import type { ContextAnchor } from "./context/projection.ts";
import { projectContext, validContextAnchor } from "./context/projection.ts";
import type { TranscriptBlock } from "./transcript-types.ts";
import type { Message } from "./types.ts";
import { assertPersistableNode } from "./sessions/codec.ts";

export const CONVERSATION_LIMITS = Object.freeze({
  nodes: 1_024,
  messageCodeUnits: 8_388_608,
  transcriptCodeUnits: 8_388_608,
  contextCodeUnits: 8_388_608,
});

export type TurnSettlement = "checkpointed" | "completed" | "failed" | "interrupted";

export type TurnFailure = Readonly<{
  text: string;
  tone: "warn" | "error";
}>;

export type TurnIdentity = Readonly<{
  providerId: string;
  model: string;
  effort: string;
}>;

export type TurnNode = Readonly<{
  id: number;
  parentId: number;
  revision: number;
  createdAt: string;
  settlement: TurnSettlement;
  identity: TurnIdentity;
  messages: readonly Message[];
  blocks: readonly TranscriptBlock[];
  context?: ContextAnchor;
  failure?: TurnFailure;
}>;

export type TurnDraft = Readonly<{
  nodeId?: number;
  parentId: number;
  createdAt: string;
  identity: TurnIdentity;
  messages: readonly Message[];
  blocks: readonly TranscriptBlock[];
  context?: ContextAnchor;
  failure?: TurnFailure;
}>;

/** Immutable tree with one selected model/transcript path. */
export class ConversationTree {
  readonly #nodes: readonly TurnNode[];
  readonly #activeNodeId: number;

  private constructor(nodes: readonly TurnNode[], activeNodeId: number) {
    this.#nodes = Object.freeze([...nodes]);
    this.#activeNodeId = activeNodeId;
    Object.freeze(this);
  }

  static empty(): ConversationTree {
    return new ConversationTree([], 0);
  }

  static restore(nodes: readonly TurnNode[], activeNodeId: number): ConversationTree {
    if (nodes.length > CONVERSATION_LIMITS.nodes) {
      throw new Error("conversation reached its session limit — start /new");
    }
    const restored: TurnNode[] = [];
    for (let index = 0; index < nodes.length; index++) {
      const node = nodes[index];
      if (node === undefined || node.id !== index + 1) {
        throw new Error("session contains a non-sequential conversation node");
      }
      const owned = ownedNode({ ...node, blocks: settledBlocks(node.blocks) });
      assertTurn(owned);
      assertPersistableNode(owned);
      restored.push(owned);
    }
    assertBounds(restored);
    if (
      !validNodeId(activeNodeId) ||
      (activeNodeId !== 0 && restored[activeNodeId - 1]?.id !== activeNodeId)
    ) throw new Error("conversation node does not exist");
    return new ConversationTree(restored, activeNodeId);
  }

  /** Commit or extend the one prospective leaf turn. */
  commit(draft: TurnDraft, settlement: TurnSettlement): ConversationTree {
    if (draft.parentId !== (draft.nodeId === undefined
      ? this.#activeNodeId
      : this.node(draft.nodeId)?.parentId)) {
      throw new Error("turn parent no longer matches the selected conversation");
    }
    if (draft.nodeId === undefined) return this.#append(draft, settlement);
    return this.#replace(draft, settlement);
  }

  select(nodeId: number): ConversationTree {
    if (!validNodeId(nodeId) || (nodeId !== 0 && this.#nodes[nodeId - 1]?.id !== nodeId)) {
      throw new Error("conversation node does not exist");
    }
    return new ConversationTree(this.#nodes, nodeId);
  }

  node(nodeId: number): TurnNode | undefined {
    return nodeId === 0 ? undefined : this.#nodes[nodeId - 1];
  }

  get nodes(): readonly TurnNode[] {
    return this.#nodes;
  }

  get activeNodeId(): number {
    return this.#activeNodeId;
  }

  get activeNode(): TurnNode | undefined {
    return this.node(this.#activeNodeId);
  }

  /** Select the newest completed turn on the active path, if one exists. */
  latestCompleted(): ConversationTree | undefined {
    let id = this.#activeNodeId;
    while (id !== 0) {
      const node = this.node(id);
      if (node === undefined) throw new Error("conversation path is incomplete");
      if (node.settlement === "completed") return this.select(id);
      id = node.parentId;
    }
    return undefined;
  }

  /** Select the newest turn that is safe to continue after a restart. */
  latestResumable(): ConversationTree | undefined {
    let id = this.#activeNodeId;
    while (id !== 0) {
      const node = this.node(id);
      if (node === undefined) throw new Error("conversation path is incomplete");
      if (node.settlement !== "checkpointed") return this.select(id);
      id = node.parentId;
    }
    return undefined;
  }

  get history(): Message[] {
    return this.#path().flatMap((node) => clone(node.messages));
  }

  get contextHistory(): Message[] {
    return projectContext(this.#path());
  }

  get transcript(): TranscriptBlock[] {
    return this.#path().flatMap((node) => [
      ...clone(node.blocks),
      ...(node.failure === undefined
        ? []
        : [{ kind: "notice" as const, text: node.failure.text, tone: node.failure.tone }]),
    ]);
  }

  #append(draft: TurnDraft, settlement: TurnSettlement): ConversationTree {
    if (this.#nodes.length >= CONVERSATION_LIMITS.nodes) {
      throw new Error("conversation reached its session limit — start /new");
    }
    const node = ownedNode({
      id: this.#nodes.length + 1,
      parentId: draft.parentId,
      revision: 1,
      createdAt: draft.createdAt,
      settlement,
      identity: draft.identity,
      messages: draft.messages,
      blocks: settledBlocks(draft.blocks),
      ...(draft.context === undefined ? {} : { context: draft.context }),
      ...(draft.failure === undefined ? {} : { failure: draft.failure }),
    });
    assertTurn(node);
    assertPersistableNode(node);
    const nodes = [...this.#nodes, node];
    assertBounds(nodes);
    return new ConversationTree(nodes, node.id);
  }

  #replace(draft: TurnDraft, settlement: TurnSettlement): ConversationTree {
    const id = draft.nodeId as number;
    const current = this.node(id);
    if (current === undefined || id !== this.#activeNodeId || this.#nodes.some((node) => node.parentId === id)) {
      throw new Error("only the active leaf turn can be checkpointed");
    }
    const node = ownedNode({
      ...current,
      revision: current.revision + 1,
      settlement,
      identity: draft.identity,
      messages: draft.messages,
      blocks: settledBlocks(draft.blocks),
      context: draft.context ?? current.context,
      failure: draft.failure,
    });
    assertTurn(node);
    assertPersistableNode(node);
    const nodes = [...this.#nodes];
    nodes[id - 1] = node;
    assertBounds(nodes);
    return new ConversationTree(nodes, id);
  }

  #path(): TurnNode[] {
    const path: TurnNode[] = [];
    let id = this.#activeNodeId;
    while (id !== 0) {
      const node = this.node(id);
      if (node === undefined) throw new Error("conversation path is incomplete");
      path.push(node);
      id = node.parentId;
    }
    path.reverse();
    return path;
  }
}

function ownedNode(node: TurnNode): TurnNode {
  return Object.freeze({
    ...node,
    identity: Object.freeze({ ...node.identity }),
    messages: Object.freeze(clone(node.messages)),
    blocks: Object.freeze(clone(node.blocks)),
    ...(node.context === undefined ? {} : { context: Object.freeze({ ...node.context }) }),
    ...(node.failure === undefined ? {} : { failure: Object.freeze({ ...node.failure }) }),
  });
}

function settledBlocks(blocks: readonly TranscriptBlock[]): TranscriptBlock[] {
  return clone(blocks).flatMap((block): TranscriptBlock[] => {
    if (block.kind === "notice") return [];
    if (block.kind === "reasoning") {
      const { live: _live, expanded: _expanded, ...settled } = block;
      return [settled];
    }
    if (block.kind === "tool") {
      if (block.tone === "pending") return [];
      const { startedAt: _startedAt, expanded: _expanded, ...settled } = block;
      return [settled];
    }
    return [block];
  });
}

function assertTurn(node: TurnNode): void {
  if (
    !validNodeId(node.id) || node.id === 0 ||
    !validNodeId(node.parentId) || node.parentId >= node.id ||
    !Number.isSafeInteger(node.revision) || node.revision < 1 ||
    node.createdAt.length === 0 || node.createdAt.length > 64 ||
    !validSettlement(node.settlement) ||
    node.messages.length < 2 || node.messages[0]?.role !== "user" ||
    node.identity.providerId.length === 0 || node.identity.providerId.length > 128 ||
    node.identity.model.length === 0 || node.identity.model.length > 512 ||
    node.identity.effort.length === 0 || node.identity.effort.length > 32
  ) throw new Error("turn checkpoint is invalid");
  if (node.settlement !== "checkpointed" && node.messages.at(-1)?.role !== "assistant") {
    throw new Error("a resumable turn must end with an assistant message");
  }
  const failed = node.settlement === "failed" || node.settlement === "interrupted";
  if (
    failed !== (node.failure !== undefined) ||
    (node.settlement === "failed" && node.failure?.tone !== "error") ||
    (node.settlement === "interrupted" && node.failure?.tone !== "warn")
  ) {
    throw new Error("turn failure state is invalid");
  }
}

function validSettlement(value: string): value is TurnSettlement {
  return value === "checkpointed" || value === "completed" ||
    value === "failed" || value === "interrupted";
}

function assertBounds(nodes: readonly TurnNode[]): void {
  let messageCodeUnits = 0;
  let transcriptCodeUnits = 0;
  let contextCodeUnits = 0;
  const contextOwners: TurnNode[] = [];
  for (const node of nodes) {
    messageCodeUnits += JSON.stringify(node.messages).length;
    transcriptCodeUnits += JSON.stringify(node.blocks).length;
    contextCodeUnits += node.context?.summary.length ?? 0;
    if (node.context !== undefined) contextOwners.push(node);
  }
  if (messageCodeUnits > CONVERSATION_LIMITS.messageCodeUnits) {
    throw new Error("conversation model history reached its session limit — start /new");
  }
  if (transcriptCodeUnits > CONVERSATION_LIMITS.transcriptCodeUnits) {
    throw new Error("conversation transcript reached its session limit — start /new");
  }
  if (contextCodeUnits > CONVERSATION_LIMITS.contextCodeUnits) {
    throw new Error("conversation context summaries reached their session limit — start /new");
  }
  assertContextPaths(nodes, contextOwners);
}

function assertContextPaths(
  nodes: readonly TurnNode[],
  owners: readonly TurnNode[],
): void {
  if (owners.length === 0) return;
  const children = Array.from({ length: nodes.length + 1 }, (): number[] => []);
  for (const node of nodes) children[node.parentId]?.push(node.id);

  const entered = new Uint32Array(nodes.length + 1);
  const exited = new Uint32Array(nodes.length + 1);
  const stack: Array<Readonly<{ id: number; exit: boolean }>> = [{ id: 0, exit: false }];
  let clock = 0;
  while (stack.length > 0) {
    const current = stack.pop() as Readonly<{ id: number; exit: boolean }>;
    if (current.exit) {
      exited[current.id] = clock++;
      continue;
    }
    entered[current.id] = clock++;
    stack.push({ id: current.id, exit: true });
    const descendants = children[current.id] as readonly number[];
    for (let index = descendants.length - 1; index >= 0; index--) {
      stack.push({ id: descendants[index] as number, exit: false });
    }
  }

  for (const owner of owners) {
    const context = owner.context as ContextAnchor;
    const boundary = nodes[context.throughNodeId - 1];
    if (boundary === undefined || !validContextAnchor(context, boundary.messages.length)) {
      throw new Error("turn context checkpoint is invalid");
    }
    if (
      entered[boundary.id] > entered[owner.id] ||
      exited[owner.id] > exited[boundary.id]
    ) throw new Error("turn context checkpoint is outside its branch");
  }
}

function validNodeId(value: number): boolean {
  return Number.isSafeInteger(value) && value >= 0;
}

function clone<T>(value: T): T {
  return structuredClone(value);
}
