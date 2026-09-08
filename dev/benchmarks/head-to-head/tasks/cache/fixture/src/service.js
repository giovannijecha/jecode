import { createCache } from './cache.js';

export function createDirectory(fetchUser, options) {
  const cache = createCache(fetchUser, options);
  return {
    user(id, options) { return cache.get(id, options); },
    changed(id) { cache.invalidate(id); },
    reset() { cache.clear(); },
  };
}
