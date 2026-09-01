// One process-local owner for durable session state.
//
// One logical conversation keeps one stable session id across every resume.
// New turns advance that session's tree head; /new is the boundary that starts
// another durable session.

import type { ConversationTree } from "../conversation.ts";
import type { SessionCatalogEntry, SessionLease } from "./store.ts";
import { DurableSessionStore } from "./store.ts";

export type ResumedSession = Readonly<{
  conversation: ConversationTree;
  persistence: SessionPersistence;
}>;

export class SessionPersistence {
  readonly #store: DurableSessionStore;
  #sessionId: string | null;
  #lease: SessionLease | undefined;
  #failure: Error | undefined;

  private constructor(
    store: DurableSessionStore,
    sessionId: string | null,
    lease?: SessionLease,
  ) {
    this.#store = store;
    this.#sessionId = sessionId;
    this.#lease = lease;
  }

  static fresh(store: DurableSessionStore): SessionPersistence {
    return new SessionPersistence(store, null);
  }

  static async resume(store: DurableSessionStore, id: string): Promise<ResumedSession> {
    const lease = await store.claim(id);
    try {
      const snapshot = await store.load(id);
      const conversation = snapshot.conversation.latestCompleted();
      if (conversation === undefined) throw new Error("session has no completed turn to resume");
      return Object.freeze({
        conversation,
        persistence: new SessionPersistence(store, id, lease),
      });
    } catch (error) {
      await lease.close();
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

  async checkpoint(conversation: ConversationTree): Promise<void> {
    if (this.#failure !== undefined) throw this.#failure;
    try {
      if (this.#sessionId === null) {
        const published = await this.#store.publish(conversation, true);
        this.#lease = published.lease;
        this.#sessionId = published.meta.id;
        return;
      }
      await this.#store.checkpoint(this.#sessionId, conversation);
    } catch (error) {
      this.#failure = error as Error;
      throw error;
    }
  }

  async reset(): Promise<void> {
    await this.#lease?.close();
    this.#lease = undefined;
    this.#sessionId = null;
    this.#failure = undefined;
  }

  async close(): Promise<void> {
    await this.#lease?.close();
    this.#lease = undefined;
  }
}
