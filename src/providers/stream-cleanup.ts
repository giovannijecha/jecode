// A protocol terminator can precede HTTP EOF. Drain a small tail off the critical
// path so fetch can reuse the connection; cancellation and broken streams abort.
export async function drainStreamTail(
  reader: ReadableStreamDefaultReader<Uint8Array>,
  signal?: AbortSignal,
): Promise<void> {
  let ended = false;
  const cancel = (): void => { void reader.cancel().catch(() => undefined); };
  const timer = setTimeout(cancel, 250);
  timer.unref();
  signal?.addEventListener("abort", cancel, { once: true });
  try {
    if (signal?.aborted) return;
    let bytes = 0;
    while (bytes <= 65_536) {
      const next = await reader.read();
      if (next.done) { ended = true; return; }
      bytes += next.value.byteLength;
    }
  } catch {
    // A completed model response remains valid even if its HTTP tail fails.
  } finally {
    clearTimeout(timer);
    signal?.removeEventListener("abort", cancel);
    if (!ended) await reader.cancel().catch(() => undefined);
    reader.releaseLock();
  }
}
