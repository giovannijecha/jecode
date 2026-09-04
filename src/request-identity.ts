// Stable provider-facing conversation identity. It is routing metadata only,
// never authorization, persistence authority, or a user-visible identifier.

import { createHash, randomUUID } from "node:crypto";
import type { Session } from "./session.ts";
import type { ConversationRequestIdentity } from "./types.ts";

const ephemeralSeeds = new WeakMap<Session, string>();

export function requestIdentityForSession(session: Session): ConversationRequestIdentity {
  let conversation = session.persistence?.conversationId;
  if (conversation === undefined) {
    conversation = ephemeralSeeds.get(session);
    if (conversation === undefined) {
      conversation = randomUUID();
      ephemeralSeeds.set(session, conversation);
    }
  }
  return identityFromSeed(`${session.provider.id}\0${conversation}`);
}

export function resetRequestIdentity(session: Session): void {
  ephemeralSeeds.delete(session);
}

function identityFromSeed(seed: string): ConversationRequestIdentity {
  const digest = createHash("sha256").update(seed).digest("hex");
  const conversationId = [
    digest.slice(0, 8),
    digest.slice(8, 12),
    `5${digest.slice(13, 16)}`,
    `${variant(digest[16] as string)}${digest.slice(17, 20)}`,
    digest.slice(20, 32),
  ].join("-");
  return Object.freeze({ conversationId, cacheKey: `jecode-${digest.slice(0, 32)}` });
}

function variant(value: string): string {
  return ((Number.parseInt(value, 16) & 0x3) | 0x8).toString(16);
}
