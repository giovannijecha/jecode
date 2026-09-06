// Reuse only the current conversation's meter. No calibration survives restart.

import type { Provider } from "../types.ts";
import { inputMeter } from "./measurement.ts";
import type { InputMeter } from "./measurement.ts";

export type InputLifetime = ReturnType<typeof inputLifetime>;

export function inputLifetime() {
  let current: { provider: Provider; conversation: string; meter: InputMeter } | undefined;
  return {
    forTurn(provider: Provider, conversation: string): InputMeter {
      if (current?.provider !== provider || current.conversation !== conversation) {
        current?.meter.reset();
        current = { provider, conversation, meter: inputMeter(provider) };
      }
      return current.meter;
    },
    reset(): void {
      current?.meter.reset();
      current = undefined;
    },
  };
}
