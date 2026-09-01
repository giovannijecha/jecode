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

export const CONVERSATION_LIMITS = Object.freeze({
  nodes: 1_024,
  messageCodeUnits: 8_388_608,
  transcriptCodeUnits: 8_388_608,
  contextCodeUnits: 8_388_608,
});

export type TurnSettlement = "checkpointed" | "completed";

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
}>;

export type TurnDraft = Readonly<{
  nodeId?: number;
  parentId: number;
  createdAt: string;
  identity: TurnIdentity;
  messages: readonly Message[];
  blocks: readonly TranscriptBlock[];
  context?: ContextAnchor;
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
    let tree = ConversationTree.empty();
    for (let index = 0; index < nodes.length; index++) {
      const node = nodes[index];
      if (node === undefined || node.id !== index + 1) {
        throw new Error("session contains a non-sequential conversation node");
      }
      tree = tree.select(node.parentId).commit({
        parentId: node.parentId,
        createdAt: node.createdAt,
        identity: node.identity,
        messages: node.messages,
        blocks: node.blocks,
        ...(node.context === undefined ? {} : { context: node.context }),
      }, node.settlement);
      const restored = tree.activeNode;
      if (restored === undefined || restored.id !== node.id) {
        throw new Error("session conversation could not be restored");
      }
      if (node.revision > 1) {
        const copy = [...tree.#nodes];
        copy[node.id - 1] = ownedNode({ ...restored, revision: node.revision });
        tree = new ConversationTree(copy, node.id);
      }
    }
    return tree.select(activeNodeId);
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

  get history(): Message[] {
    return this.#path().flatMap((node) => clone(node.messages));
  }

  get contextHistory(): Message[] {
    return projectContext(this.#path());
  }

  get transcript(): TranscriptBlock[] {
    return this.#path().flatMap((node) => clone(node.blocks));
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
    });
    assertTurn(node);
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
    });
    assertTurn(node);
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
    (node.settlement !== "checkpointed" && node.settlement !== "completed") ||
    node.messages.length < 2 || node.messages[0]?.role !== "user" ||
    node.identity.providerId.length === 0 || node.identity.providerId.length > 128 ||
    node.identity.model.length === 0 || node.identity.model.length > 512 ||
    node.identity.effort.length === 0 || node.identity.effort.length > 32
  ) throw new Error("turn checkpoint is invalid");
  if (node.settlement === "completed" && node.messages.at(-1)?.role !== "assistant") {
    throw new Error("a completed turn must end with an assistant message");
  }
}

function assertBounds(nodes: readonly TurnNode[]): void {
  let messageCodeUnits = 0;
  let transcriptCodeUnits = 0;
  let contextCodeUnits = 0;
  for (const node of nodes) {
    messageCodeUnits += JSON.stringify(node.messages).length;
    transcriptCodeUnits += JSON.stringify(node.blocks).length;
    contextCodeUnits += node.context?.summary.length ?? 0;
    if (node.context !== undefined) assertContextPath(nodes, node);
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
}

function assertContextPath(nodes: readonly TurnNode[], owner: TurnNode): void {
  const context = owner.context as ContextAnchor;
  const boundary = nodes[context.throughNodeId - 1];
  if (boundary === undefined || !validContextAnchor(context, boundary.messages.length)) {
    throw new Error("turn context checkpoint is invalid");
  }
  let id = owner.id;
  while (id !== 0 && id !== boundary.id) id = nodes[id - 1]?.parentId ?? 0;
  if (id !== boundary.id) throw new Error("turn context checkpoint is outside its branch");
}

function validNodeId(value: number): boolean {
  return Number.isSafeInteger(value) && value >= 0;
}

function clone<T>(value: T): T {
  return structuredClone(value);
}
