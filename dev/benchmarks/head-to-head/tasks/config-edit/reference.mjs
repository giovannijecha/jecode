// Evaluator calibration only. Never copied into a participant workspace.
import * as fs from 'node:fs/promises';
import path from 'node:path';
import { randomUUID } from 'node:crypto';
import { isDeepStrictEqual } from 'node:util';
const unsafe = key => ['__proto__', 'prototype', 'constructor'].includes(key);
export function parsePointer(value) {
  if (typeof value !== 'string' || (value !== '' && !value.startsWith('/'))) throw new Error('pointer');
  if (value === '') return [];
  return value.slice(1).split('/').map(part => {
    if (/~(?![01])/u.test(part)) throw new Error('escape');
    const key = part.replace(/~[01]/gu, match => match === '~0' ? '~' : '/');
    if (unsafe(key)) throw new Error('unsafe');
    return key;
  });
}
function copy(value, ancestors = new Set()) {
  if (value === null || typeof value === 'string' || typeof value === 'boolean') return value;
  if (typeof value === 'number' && Number.isFinite(value)) return value;
  if (typeof value !== 'object' || ancestors.has(value)) throw new Error('JSON value');
  if (!Array.isArray(value) && Object.getPrototypeOf(value) !== Object.prototype && Object.getPrototypeOf(value) !== null) throw new Error('plain object');
  if (Reflect.ownKeys(value).some(key => typeof key !== 'string' || unsafe(key))) throw new Error('unsafe key');
  ancestors.add(value);
  let output;
  if (Array.isArray(value)) {
    output = [];
    for (let i = 0; i < value.length; i++) {
      if (!Object.hasOwn(value, i)) throw new Error('hole');
      output.push(copy(value[i], ancestors));
    }
  } else {
    output = {};
    for (const key of Object.keys(value)) output[key] = copy(value[key], ancestors);
  }
  ancestors.delete(value);
  return output;
}
function index(parent, key, append) {
  if (!Array.isArray(parent)) return key;
  if (append && key === '-') return parent.length;
  if (!/^(0|[1-9][0-9]*)$/u.test(key)) throw new Error('index');
  const number = Number(key);
  if (!Number.isSafeInteger(number) || number > parent.length || (!append && number === parent.length)) throw new Error('range');
  return number;
}
export function patchDocument(document, operations) {
  let output = copy(document);
  if (!Array.isArray(operations)) throw new Error('operations');
  for (const operation of operations) {
    if (!operation || typeof operation !== 'object' || Array.isArray(operation) ||
      Object.keys(operation).some(key => !['op','path','value'].includes(key)) ||
      !['set','remove','test'].includes(operation.op)) throw new Error('operation');
    const { op } = operation, parts = parsePointer(operation.path);
    const value = op === 'remove' ? undefined : copy(operation.value);
    if (!parts.length) {
      if (op === 'remove') throw new Error('root removal');
      if (op === 'set') output = value;
      else if (!isDeepStrictEqual(output, value)) throw new Error('test');
      continue;
    }
    let parent = output;
    for (const key of parts.slice(0, -1)) {
      if (!parent || typeof parent !== 'object') throw new Error('parent');
      const target = index(parent, key, false);
      if (!Object.hasOwn(parent, target)) throw new Error('parent missing');
      parent = parent[target];
    }
    if (!parent || typeof parent !== 'object') throw new Error('parent');
    const key = index(parent, parts.at(-1), op === 'set');
    if (op !== 'set' && !Object.hasOwn(parent, key)) throw new Error('missing');
    if (op === 'set') parent[key] = value;
    else if (op === 'remove') { if (Array.isArray(parent)) parent.splice(key, 1); else delete parent[key]; }
    else if (!isDeepStrictEqual(parent[key], value)) throw new Error('test');
  }
  return output;
}
async function target(root, relative) {
  let current = root;
  for (const part of relative.split(path.sep)) {
    current = path.join(current, part);
    const stat = await fs.lstat(current);
    if (stat.isSymbolicLink()) throw new Error('symlink');
  }
  const stat = await fs.lstat(current);
  if (!stat.isFile()) throw new Error('regular file');
  return { file: current, mode: stat.mode & 0o777 };
}
export async function migrateFiles(root, changes, { dryRun = false, signal, onCommit } = {}) {
  signal?.throwIfAborted();
  root = await fs.realpath(root);
  if (!(await fs.stat(root)).isDirectory() || !Array.isArray(changes)) throw new Error('root or changes');
  const entries = [], seen = new Set();
  for (const change of changes) {
    if (!change || typeof change.path !== 'string' || path.isAbsolute(change.path) || change.path.split(/[\\/]/u).includes('..')) throw new Error('path');
    const relative = path.normalize(change.path);
    if (seen.has(relative)) throw new Error('duplicate');
    seen.add(relative);
    const info = await target(root, relative), before = await fs.readFile(info.file);
    const raw = before.toString('utf8'), bom = raw.startsWith('\uFEFF');
    const document = JSON.parse(bom ? raw.slice(1) : raw), next = patchDocument(document, change.operations);
    if (!isDeepStrictEqual(document, next)) entries.push({ ...info, relative, before,
      after: (bom ? '\uFEFF' : '') + JSON.stringify(next, null, 2) + '\n' });
  }
  const report = { changed: entries.map(entry => entry.relative) };
  if (dryRun) return report;
  const staged = [], committed = [];
  try {
    for (const entry of entries) {
      signal?.throwIfAborted();
      const stage = entry.file + '.' + randomUUID() + '.stage';
      staged.push(stage); entry.stage = stage;
      await fs.writeFile(stage, entry.after, { flag: 'wx', mode: entry.mode }); await fs.chmod(stage, entry.mode);
    }
    for (const entry of entries) {
      await target(root, entry.relative);
      if (!(await fs.readFile(entry.file)).equals(entry.before)) throw new Error('target changed');
    }
    for (const entry of entries) {
      signal?.throwIfAborted();
      await target(root, entry.relative);
      if (!(await fs.readFile(entry.file)).equals(entry.before)) throw new Error('target changed');
      await fs.rename(entry.stage, entry.file); committed.push(entry);
      await onCommit?.(entry.relative, committed.length - 1);
      signal?.throwIfAborted();
    }
    return report;
  } catch (error) {
    for (const entry of committed.reverse()) {
      const restore = entry.file + '.' + randomUUID() + '.restore'; staged.push(restore);
      await fs.writeFile(restore, entry.before, { flag: 'wx', mode: entry.mode }); await fs.chmod(restore, entry.mode);
      await fs.rename(restore, entry.file);
    }
    throw error;
  } finally {
    for (const stage of staged) await fs.rm(stage, { force: true });
  }
}
