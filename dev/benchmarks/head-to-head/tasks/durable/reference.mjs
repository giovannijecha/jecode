// Offline reference for validating the evaluator. Never copied to a task workspace.
import { open, lstat, rename, unlink } from 'node:fs/promises';
import { constants } from 'node:fs';
import { randomUUID } from 'node:crypto';
import { dirname, join } from 'node:path';
import { pathToFileURL } from 'node:url';

export async function reference(workspace) {
  const { planBuild } = await import(pathToFileURL(join(workspace, 'src/index.js')));
  const plain = x => x !== null && typeof x === 'object' && [null, Object.prototype].includes(Object.getPrototypeOf(x));
  const fields = (x, names) => plain(x) && Reflect.ownKeys(x).length === names.length && names.every(n => Object.hasOwn(x, n));
  const copyTask = t => ({ id: t.id, deps: [...t.deps], duration: t.duration });
  function definition(document) {
    planBuild(document);
    return document.tasks.map(t => ({ ...copyTask(t), deps: [...t.deps].sort() }))
      .sort((a, b) => a.id < b.id ? -1 : a.id > b.id ? 1 : 0);
  }
  function validateSnapshot(value, expected) {
    if (!fields(value, ['version', 'tasks', 'completed']) || value.version !== 1 || !Array.isArray(value.completed)) throw new Error('invalid checkpoint');
    const tasks = definition({ tasks: value.tasks });
    const completed = [...value.completed];
    if (completed.some(id => typeof id !== 'string') || JSON.stringify(completed) !== JSON.stringify([...new Set(completed)].sort())) throw new Error('invalid completed');
    if (JSON.stringify(value.tasks.map(copyTask)) !== JSON.stringify(tasks)) throw new Error('noncanonical definition');
    if (expected && JSON.stringify(expected) !== JSON.stringify(tasks)) throw new Error('definition mismatch');
    const byId = new Map(tasks.map(t => [t.id, t])); const known = new Set(completed);
    for (const id of completed) {
      if (!byId.has(id) || byId.get(id).deps.some(dep => !known.has(dep))) throw new Error('inconsistent completion');
    }
    return { version: 1, tasks, completed };
  }
  async function executePlan(document, options) {
    const tasks = definition(document);
    if (!plain(options) || Reflect.ownKeys(options).some(k => !['run','concurrency','signal','checkpoint'].includes(k)) || typeof options.run !== 'function') throw new Error('invalid options');
    const { run, signal, checkpoint } = options;
    const concurrency = options.concurrency ?? 2;
    if (!Number.isInteger(concurrency) || concurrency < 1 || concurrency > 32 ||
        (signal !== undefined && !(signal instanceof AbortSignal)) ||
        (checkpoint !== undefined && (!checkpoint || typeof checkpoint.load !== 'function' || typeof checkpoint.save !== 'function'))) throw new Error('invalid options');
    const control = new AbortController();
    const abort = () => control.abort(signal.reason);
    signal?.addEventListener('abort', abort, { once: true });
    if (signal?.aborted) abort();
    const completed = new Set(), failed = new Set(), blocked = new Set();
    const byId = new Map(tasks.map(t => [t.id, t]));
    const children = new Map(tasks.map(t => [t.id, []]));
    for (const t of tasks) for (const dep of t.deps) children.get(dep).push(t.id);
    const active = new Map(); let fatal, saveTail = Promise.resolve();
    const snap = ids => ({ version: 1, tasks: tasks.map(copyTask), completed: [...ids].sort() });
    function failStorage(error) { if (fatal === undefined) fatal = { error }; control.abort(error); }
    async function commit(id) {
      const next = saveTail.then(async () => {
        if (fatal) return;
        try {
          if (checkpoint) await checkpoint.save(snap(new Set([...completed, id])));
          completed.add(id);
        } catch (error) { failStorage(error); }
      }); saveTail = next; await next;
    }
    function block(id) {
      const stack = [...children.get(id)];
      while (stack.length) {
        const next = stack.pop(); if (blocked.has(next)) continue;
        blocked.add(next); stack.push(...children.get(next));
      }
    }
    try {
      if (checkpoint && !signal?.aborted) {
        const state = await checkpoint.load();
        if (state === null) await checkpoint.save(snap([]));
        else for (const id of validateSnapshot(state, tasks).completed) completed.add(id);
      }
      const pending = new Set(tasks.filter(t => !completed.has(t.id)).map(t => t.id));
      while (pending.size || active.size) {
        for (const id of pending) {
          if (blocked.has(id)) { pending.delete(id); continue; }
          if (control.signal.aborted || active.size >= concurrency) break;
          if (byId.get(id).deps.some(dep => !completed.has(dep))) continue;
          pending.delete(id);
          const attempt = (async () => {
            try { await run(copyTask(byId.get(id)), { signal: control.signal }); }
            catch {
              if (!fatal && !signal?.aborted) { failed.add(id); block(id); }
              return;
            }
            if (!fatal) await commit(id);
          })().finally(() => active.delete(id));
          active.set(id, attempt);
        }
        if (active.size) await Promise.race(active.values()); else break;
      }
      await saveTail;
      if (fatal) throw fatal.error;
      const sorted = set => [...set].sort();
      return { status: signal?.aborted ? 'cancelled' : failed.size ? 'failed' : 'completed',
        completed: sorted(completed), failed: sorted(failed), blocked: sorted(blocked),
        pending: tasks.map(t => t.id).filter(id => !completed.has(id) && !failed.has(id) && !blocked.has(id)) };
    } finally { signal?.removeEventListener('abort', abort); }
  }
  function createCheckpointStore(file) {
    if (typeof file !== 'string' || !file.trim()) throw new Error('invalid path');
    const limit = 1024 * 1024; let queue = Promise.resolve();
    async function target() {
      try { const stat = await lstat(file); if (!stat.isFile() || stat.isSymbolicLink()) throw new Error('not a regular file'); }
      catch (error) { if (error.code !== 'ENOENT') throw error; }
    }
    return {
      async load() {
        await target(); let handle;
        try { handle = await open(file, constants.O_RDONLY | (constants.O_NOFOLLOW ?? 0)); }
        catch (error) { if (error.code === 'ENOENT') return null; throw error; }
        try {
          const info = await handle.stat(); if (!info.isFile() || info.size > limit) throw new Error('file limit or type');
          const bytes = Buffer.alloc(limit + 1); let size = 0;
          while (size <= limit) { const read = await handle.read(bytes, size, bytes.length - size); if (!read.bytesRead) break; size += read.bytesRead; }
          if (size > limit) throw new Error('file limit');
          return validateSnapshot(JSON.parse(new TextDecoder('utf-8', { fatal: true }).decode(bytes.subarray(0, size))));
        } finally { await handle.close(); }
      },
      save(value) {
        let bytes;
        try { bytes = JSON.stringify(validateSnapshot(value))+'\n'; if (Buffer.byteLength(bytes) > limit) throw new Error('file limit'); }
        catch (error) { return Promise.reject(error); }
        const next = queue.catch(() => {}).then(async () => {
          await target(); const temporary = join(dirname(file), `.checkpoint-${randomUUID()}`); let handle;
          try {
            handle = await open(temporary, 'wx', 0o600); await handle.writeFile(bytes); await handle.sync(); await handle.close(); handle = undefined;
            await target(); await rename(temporary, file);
          } finally { await handle?.close(); await unlink(temporary).catch(error => { if (error.code !== 'ENOENT') throw error; }); }
        }); queue = next; return next;
      },
    };
  }
  return { executePlan, createCheckpointStore };
}
