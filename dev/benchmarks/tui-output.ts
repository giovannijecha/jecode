// A counting output sink; optional throttling models a slow consumer, not a display.

import { Writable } from "node:stream";
import { finished } from "node:stream/promises";

export function measuredOutput(bytesPerSecond?: number) {
  if (bytesPerSecond !== undefined && (!Number.isFinite(bytesPerSecond) || bytesPerSecond <= 0)) {
    throw new Error("output rate must be a positive finite number");
  }
  let bytes = 0;
  let writes = 0;
  let peakQueuedBytes = 0;
  let backpressureWrites = 0;
  let timer: NodeJS.Timeout | undefined;
  const stream = new Writable({
    highWaterMark: 4_096,
    write(chunk: Buffer, _encoding, callback) {
      if (bytesPerSecond === undefined) callback();
      else timer = setTimeout(() => {
        timer = undefined;
        callback();
      }, Math.ceil(chunk.byteLength * 1_000 / bytesPerSecond));
    },
    destroy(error, callback) {
      if (timer !== undefined) clearTimeout(timer);
      callback(error);
    },
  });
  return {
    write(text: string, acknowledged: () => void): number {
      const size = Buffer.byteLength(text);
      bytes += size;
      writes++;
      if (!stream.write(text, acknowledged)) backpressureWrites++;
      peakQueuedBytes = Math.max(peakQueuedBytes, stream.writableLength);
      return size;
    },
    stats: () => ({ bytes, writes, peakQueuedBytes, backpressureWrites }),
    ready: () => !stream.writableNeedDrain,
    onReady(handler: () => void): () => void {
      stream.on("drain", handler);
      return () => { stream.off("drain", handler); };
    },
    async close(): Promise<void> {
      const done = finished(stream, { cleanup: true });
      stream.end();
      await done;
    },
    destroy: () => stream.destroy(),
  };
}
