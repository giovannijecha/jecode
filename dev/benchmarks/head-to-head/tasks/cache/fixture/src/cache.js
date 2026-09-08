export function createCache(loader, { ttlMs = 1000, now = Date.now } = {}) {
  const values = new Map();
  const pending = new Map();
  return {
    async get(key, { signal } = {}) {
      if (signal?.aborted) throw signal.reason;
      const cached = values.get(key);
      if (cached?.value && cached.expires > now()) return cached.value;
      if (pending.has(key)) return pending.get(key);
      const expires = now() + ttlMs;
      const promise = Promise.resolve(loader(key)).then(value => {
        values.set(key, { value, expires });
        return value;
      }).finally(() => pending.delete(key));
      pending.set(key, promise);
      return promise;
    },
    invalidate(key) { values.delete(key); },
    clear() { values.clear(); },
  };
}
