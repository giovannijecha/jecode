// Display boundaries between Responses summary parts; provider replay stays untouched.

type SummaryDelta = {
  delta?: unknown;
  item_id?: unknown;
  output_index?: unknown;
  summary_index?: unknown;
};
type Part = { item?: string; output?: number; summary?: number };

export class OpenAISummary {
  #part: Part | undefined;
  #ended = false;
  #tail = "";

  reset(): void {
    this.#part = undefined;
    this.#ended = false;
    this.#tail = "";
  }

  end(): void { this.#ended = true; }

  delta(event: SummaryDelta): string | undefined {
    if (typeof event.delta !== "string" || event.delta === "") return undefined;
    const part: Part = {
      item: typeof event.item_id === "string" ? event.item_id : undefined,
      output: index(event.output_index),
      summary: index(event.summary_index),
    };
    const previous = this.#part;
    const changed = previous !== undefined && (["item", "output", "summary"] as const).some(key => (
      part[key] !== undefined && previous[key] !== undefined && part[key] !== previous[key]
    ));
    // Completion events cover older/idless streams. Identity changes also cover
    // streams that omit those events. Wait for text so empty parts add no rows.
    const boundary = previous !== undefined && (this.#ended || changed);
    const trailing = this.#tail.match(/\n{0,2}$/u)?.[0].length ?? 0;
    const leading = event.delta.match(/^\n{0,2}/u)?.[0].length ?? 0;
    const prefix = boundary ? "\n".repeat(Math.max(0, 2 - trailing - leading)) : "";
    const text = prefix + event.delta;
    this.#tail = (this.#tail + text).slice(-2);
    this.#part = part;
    this.#ended = false;
    return text;
  }
}

function index(value: unknown): number | undefined {
  return typeof value === "number" && Number.isSafeInteger(value) && value >= 0 ? value : undefined;
}
