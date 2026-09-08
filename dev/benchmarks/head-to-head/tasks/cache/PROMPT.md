Fix the asynchronous cache's race conditions. Complete the implementation,
regression tests and README. Use one agent and Node's standard library only.
Work only in this project: no dependencies, web, external services, delegated
agents or Git history changes. Do not inspect sibling directories.

Keep `createCache(loader, { ttlMs = 1000, now = Date.now } = {})` in
`src/cache.js`, returning `get(key, { signal } = {})`, `invalidate(key)` and
`clear()`. The service in `src/service.js` must remain compatible.

Required behavior:

1. Keys are strings, including empty strings and `__proto__`. `get` returns a
   promise. Concurrent gets for the same live generation call the loader once;
   different keys load independently. Cache every resolved value, including
   undefined, false and null. Synchronous loader exceptions become rejections.
   A rejected load is evicted so a later get retries.
2. A successful value expires `ttlMs` milliseconds after resolution, not after
   the load starts. At the exact expiry it is stale. A pending load does not
   expire. `ttlMs = 0` still coalesces concurrent pending gets but retains no
   resolved value. Constructor validation rejects non-functions for loader/now
   and negative, non-finite or non-number TTLs. Assume `now()` is a finite,
   monotonically nondecreasing number. Reject non-string keys without loading.
3. `invalidate(key)` and `clear()` detach pending as well as resolved entries.
   Existing callers still receive the old generation's eventual result or
   error. A new get starts a new generation immediately. Neither late success
   nor late failure from an old generation may overwrite or evict its replacement.
4. Cancellation is per caller. An already-aborted signal rejects with its exact
   `reason` and must not start a load, even on a cache hit. Aborting one waiter
   rejects that waiter promptly and leaves other waiters and the shared load
   alive. A sole cancelled waiter also leaves the load eligible to populate the
   cache. Remove each caller's abort listener on settlement; do not produce
   unhandled rejections when a detached/cancelled load later fails. Callers pass
   either no signal or a real AbortSignal.
5. Preserve the service API and its cache invalidation behavior. Add deterministic
   tests using deferred promises and an injected clock rather than sleep-based
   race tests. Cover invalidation, clear, failures, TTL and waiter cancellation.
   Update the README, run all tests, review the changes and fix issues you find.

Briefly report the changes and the checks actually performed. The contract above
defines the task; no clarification is needed.
