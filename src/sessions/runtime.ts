// One process-local owner for durable session state.
//
// One logical conversation keeps one stable session id across every resume.
// New turns advance that session's tree head; /new is the boundary that starts
// another durable session.

import type { ConversationTree } from "../conversation.ts";
import type { SessionCatalogEntry, SessionLease, SessionSnapshot } from "./store.ts";
import { DurableSessionStore, reserveSessionId } from "./store.ts";

export type ResumedSession = Readonly<{
  conversation: ConversationTree;
  persistence: SessionPersistence;
}>;

export class SessionPersistence {
  readonly #store: DurableSessionStore;
  #sessionId: string | null;
  #conversationId: string;
  #lease: SessionLease | undefined;
  #snapshot: SessionSnapshot | undefined;
  #failure: Error | undefined;

  private constructor(
    store: DurableSessionStore,
    sessionId: string | null,
    lease?: SessionLease,
    snapshot?: SessionSnapshot,
    conversationId: string = sessionId ?? reserveSessionId(),
  ) {
    this.#store = store;
    this.#sessionId = sessionId;
    this.#conversationId = conversationId;
    this.#lease = lease;
    this.#snapshot = snapshot;
  }

  static fresh(store: DurableSessionStore): SessionPersistence {
    return new SessionPersistence(store, null, undefined, undefined, reserveSessionId());
  }

  static async resume(store: DurableSessionStore, id: string): Promise<ResumedSession> {
    const lease = await store.claim(id);
    try {
      const snapshot = await store.load(id, lease);
      const conversation = snapshot.conversation.latestResumable();
      if (conversation === undefined) throw new Error("session has no resumable turn");
      return Object.freeze({
        conversation,
        persistence: new SessionPersistence(store, id, lease, snapshot, id),
      });
    } catch (error) {
      await lease.close().catch(() => undefined);
      throw error;
    }
  }

  static async candidates(store: DurableSessionStore): Promise<SessionCatalogEntry[]> {
    return (await store.list()).filter((entry) => !entry.active);
  }

  get failure(): Error | undefined {
    return this.#failure;
  }

  get sessionId(): string | null {
    return this.#sessionId;
  }

  /** Reserved before publication so provider cache identity survives resume. */
  get conversationId(): string {
    return this.#conversationId;
  }

  async checkpoint(conversation: ConversationTree): Promise<void> {
    if (this.#failure !== undefined) throw this.#failure;
    try {
      if (this.#sessionId === null) {
        const published = await this.#store.publish(conversation, true, this.#conversationId);
        this.#lease = published.lease;
        this.#sessionId = published.meta.id;
        this.#snapshot = published;
        return;
      }
      if (this.#lease === undefined || this.#snapshot === undefined) {
        throw new Error("session persistence has no verified owner snapshot");
      }
      await this.#lease.assertOwned();
      this.#snapshot = await this.#store.checkpoint(this.#snapshot, conversation, this.#lease);
    } catch (error) {
      this.#failure = error as Error;
      throw error;
    }
  }

  async reset(): Promise<void> {
    await this.#lease?.close();
    this.#lease = undefined;
    this.#sessionId = null;
    this.#conversationId = reserveSessionId();
    this.#snapshot = undefined;
    this.#failure = undefined;
  }

  async close(): Promise<void> {
    await this.#lease?.close();
    this.#lease = undefined;
  }
}
