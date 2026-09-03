// A bounded cooperative inbox for guidance submitted during an active turn.

export const MAX_STEERING_MESSAGES = 8;
export const MAX_STEERING_CODE_UNITS = 32_768;

export type SteeringOffer = "queued" | "full" | "closed";

export type SteeringBatch = Readonly<{
  messages: readonly string[];
  closed: boolean;
}>;

export type SteeringSource = {
  drain(): readonly string[];
  drainOrClose(): SteeringBatch;
};

export type SteeringInbox = SteeringSource & {
  offer(text: string): SteeringOffer;
  close(): readonly string[];
  readonly pending: number;
  readonly accepting: boolean;
};

/**
 * Build one inbox for one model turn. `drainOrClose` is the atomic completion
 * handshake: pending guidance keeps the turn open; an empty queue closes it
 * before the final checkpoint so late input cannot disappear.
 */
export function steeringInbox(
  changed: (pending: number, accepting: boolean) => void = () => {},
): SteeringInbox {
  let accepting = true;
  let codeUnits = 0;
  const messages: string[] = [];

  const take = (): string[] => {
    if (messages.length === 0) return [];
    const drained = messages.splice(0);
    codeUnits = 0;
    changed(0, accepting);
    return drained;
  };

  return {
    offer(text) {
      if (!accepting) return "closed";
      if (
        messages.length >= MAX_STEERING_MESSAGES ||
        codeUnits + text.length > MAX_STEERING_CODE_UNITS
      ) return "full";
      messages.push(text);
      codeUnits += text.length;
      changed(messages.length, true);
      return "queued";
    },
    drain: take,
    drainOrClose() {
      if (messages.length > 0) return { messages: take(), closed: false };
      accepting = false;
      changed(0, false);
      return { messages: [], closed: true };
    },
    close() {
      if (!accepting && messages.length === 0) return [];
      accepting = false;
      if (messages.length === 0) {
        changed(0, false);
        return [];
      }
      return take();
    },
    get pending() {
      return messages.length;
    },
    get accepting() {
      return accepting;
    },
  };
}
