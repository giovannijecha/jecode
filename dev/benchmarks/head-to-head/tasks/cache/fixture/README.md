# Cached directory

`createCache(loader, options)` coalesces async lookups. `createDirectory(fetchUser)`
offers `user(id, options)`, `changed(id)` and `reset()` around that cache.

Run `npm test`. The current smoke tests only cover successful sequential reads.
