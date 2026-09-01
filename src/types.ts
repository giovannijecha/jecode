// The normalized vocabulary the controller speaks. Deliberately smaller than
// any vendor's wire format: text, a tool call, a tool result. Providers
// translate to and from this; the controller never sees a vendor shape.

export type Role = "user" | "assistant";

export type TextBlock = { kind: "text"; text: string };

export type ToolCallBlock = {
  kind: "tool_call";
  id: string;
  name: string;
  input: Record<string, unknown>;
};

export type ToolResultBlock = {
  kind: "tool_result";
  id: string;
  output: string;
  isError: boolean;
};

export type Block = TextBlock | ToolCallBlock | ToolResultBlock;

/** Token accounting normalized across providers. Missing vendor fields are 0. */
export type Usage = {
  inputTokens: number;
  outputTokens: number;
  cachedInputTokens: number;
  cacheWriteInputTokens: number;
  reasoningTokens: number;
};

// `raw` carries the provider's own content for an assistant turn.
//
// Some providers require their blocks echoed back verbatim on the next
// request (Anthropic thinking blocks, OpenAI reasoning items) and those have
// no representation in the vocabulary above. So a provider re-serializing
// history uses `raw` when `rawFrom` is its own id, and rebuilds from the
// normalized blocks otherwise. That fallback is also what makes switching
// provider mid-conversation degrade instead of erroring.
export type Message = {
  role: Role;
  content: Block[];
  raw?: unknown;
  rawFrom?: string;
  /** Usage for the request that produced this assistant message. */
  usage?: Usage;
};

export type ToolSpec = {
  name: string;
  description: string;
  /** JSON Schema for the tool's arguments. Always an object schema. */
  input: Record<string, unknown>;
};

/**
 * What arrives while a response is still being generated. Display only — the
 * authoritative message is the one `send` resolves to, and nothing here is
 * ever appended to the history.
 */
export type StreamEvent =
  | { kind: "text"; text: string }
  | { kind: "thinking"; text: string };

export type SendRequest = {
  model: string;
  system: string;
  messages: Message[];
  tools: ToolSpec[];
  maxTokens: number;
  effort: string;
  signal?: AbortSignal;
  onStream?: (event: StreamEvent) => void;
  /** Provider and HTTP lifecycle updates that are useful on a live screen. */
  onStatus?: (status: string) => void;
};

export type ProviderAuth =
  | { kind: "api-key"; keyVar: string }
  | { kind: "oauth"; account: "openai-codex"; label: string };

/** Provider-advertised input capacity for one model. */
export type ModelContextWindow = Readonly<{
  /** Input tokens available after provider-specific headroom is reserved. */
  tokens: number;
  /** Optional provider safety ceiling for automatic compaction. */
  compactAtTokens?: number;
}>;

export type Provider = {
  readonly id: string;
  readonly defaultModel: string;
  /**
   * How this provider proves who the user is. The controller never inspects
   * it; commands use the metadata to open the right authentication flow.
   */
  readonly auth: ProviderAuth;
  /** Where prompts leave the machine for the current provider configuration. */
  location?(): "cloud" | "local";
  /** What stands between this provider and a request, if anything does. */
  blocked(): string | undefined;
  /** What it will answer to, asked of it rather than remembered here. */
  models(signal?: AbortSignal, onStatus?: (status: string) => void): Promise<string[]>;
  /** Reasoning depths accepted by one model; an empty list means model-controlled. */
  efforts?(
    model: string,
    signal?: AbortSignal,
    onStatus?: (status: string) => void,
  ): Promise<readonly string[]>;
  /** Best available context capacity for one model; absence uses a safe fallback. */
  contextWindow?(
    model: string,
    signal?: AbortSignal,
    onStatus?: (status: string) => void,
  ): Promise<ModelContextWindow | undefined>;
  send(req: SendRequest): Promise<Message>;
};

export function isToolCall(block: Block): block is ToolCallBlock {
  return block.kind === "tool_call";
}

export function isText(block: Block): block is TextBlock {
  return block.kind === "text";
}
