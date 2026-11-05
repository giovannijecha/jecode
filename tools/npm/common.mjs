import {execFileSync, spawn} from 'node:child_process';
import {createHash} from 'node:crypto';
import fs from 'node:fs';
import path from 'node:path';
import {createRequire} from 'node:module';
import {fileURLToPath} from 'node:url';

export const root = fileURLToPath(new URL('../../', import.meta.url));
export const packageName = '@giovannijecha/jecode';
export const {targets} = createRequire(import.meta.url)('../../npm/targets.cjs');
export const host = `${process.platform}-${process.arch}`;

export function version() {
  const cargo = fs.readFileSync(path.join(root, 'Cargo.toml'), 'utf8');
  const section = cargo.match(/^\[package\]\s*\n([\s\S]*?)(?=^\[|$(?![\s\S]))/m)?.[1];
  const value = section?.match(/^version\s*=\s*"([^"]+)"\s*$/m)?.[1];
  if (!value || !/^\d+\.\d+\.\d+(?:-[\w.-]+)?$/.test(value)) {
    throw new Error('Cargo.toml must contain one supported package version.');
  }
  return value;
}

export function revision() {
  return execFileSync('git', ['rev-parse', 'HEAD'], {cwd: root, encoding: 'utf8'}).trim();
}

export function sourceClean() {
  return execFileSync('git', ['status', '--porcelain', '--untracked-files=normal'], {
    cwd: root, encoding: 'utf8',
  }).trim() === '';
}

export function digest(bytes) {
  return createHash('sha256').update(bytes).digest('hex');
}

export function regularFile(file) {
  if (!fs.lstatSync(file).isFile()) throw new Error(`Expected a regular file: ${file}`);
  return fs.readFileSync(file);
}

export function exceeds(version, ceiling) {
  const left = version.split('.').map(Number), right = ceiling.split('.').map(Number);
  for (let i = 0; i < Math.max(left.length, right.length); i++) {
    if ((left[i] ?? 0) !== (right[i] ?? 0)) return (left[i] ?? 0) > (right[i] ?? 0);
  }
  return false;
}

// The native helpers a release executable carries, by name. Cargo keeps stale
// build outputs, so only files whose exact bytes are embedded are returned.
export function embeddedFiles(binary, mainBytes, isNative) {
  const build = path.join(path.dirname(binary), 'build');
  if (!fs.existsSync(build)) throw new Error('Stage release binaries from their Cargo release output directory.');
  const files = new Map();
  for (const entry of fs.readdirSync(build, {withFileTypes: true})) {
    if (!entry.isDirectory() || !entry.name.startsWith('jecode-')) continue;
    const output = path.join(build, entry.name, 'out');
    if (!fs.existsSync(output)) continue;
    for (const helper of fs.readdirSync(output, {withFileTypes: true})) {
      if (!helper.isFile() || !helper.name.startsWith('jecode-')) continue;
      const file = path.join(output, helper.name), bytes = fs.readFileSync(file);
      if (isNative(bytes) && mainBytes.indexOf(bytes) >= 0) files.set(helper.name, file);
    }
  }
  return files;
}

export function requireFiles(files, expected, platform) {
  for (const name of expected) {
    if (!files.has(name)) throw new Error(`Cannot audit embedded ${platform} helper ${name}; rebuild the release first.`);
  }
}

export function freshDirectory(directory) {
  const absolute = path.resolve(directory);
  fs.mkdirSync(path.dirname(absolute), {recursive: true});
  fs.mkdirSync(absolute);
  return absolute;
}

export function json(file, value) {
  fs.writeFileSync(file, `${JSON.stringify(value, null, 2)}\n`, {flag: 'wx'});
}

// Call the npm CLI directly on Windows, avoiding cmd.exe quoting and shims.
export function npmCommand(args) {
  const candidate = process.env.npm_execpath ||
    path.join(path.dirname(process.execPath), 'node_modules/npm/bin/npm-cli.js');
  if (fs.existsSync(candidate)) return [process.execPath, candidate, ...args];
  if (process.platform !== 'win32') return ['npm', ...args];
  throw new Error('Cannot find npm beside Node. Run this command through an npm-provided Node installation.');
}

export function command(executable, args, options = {}) {
  return new Promise((resolve, reject) => {
    const input = options.input ?? '';
    const child = spawn(executable, args, {
      cwd: root, windowsHide: true, ...options,
      stdio: [input.length ? 'pipe' : 'ignore', 'pipe', 'pipe'],
    });
    let stdout = '', stderr = '';
    let inputError;
    const timer = setTimeout(() => child.kill(), options.timeout ?? 120_000);
    child.stdout.setEncoding('utf8').on('data', chunk => { stdout += chunk; });
    child.stderr.setEncoding('utf8').on('data', chunk => { stderr += chunk; });
    child.on('error', error => { clearTimeout(timer); reject(error); });
    child.on('close', (status, signal) => {
      clearTimeout(timer);
      if (inputError) { reject(inputError); return; }
      resolve({status, signal, stdout, stderr});
    });
    // A child can close stdin before consuming a payload. Stop and reap it
    // before rejecting, instead of leaving an unhandled stream error.
    child.stdin?.on('error', error => { inputError = error; child.kill(); });
    child.stdin?.end(input);
  });
}

export async function npm(args, options = {}) {
  const [executable, ...forwarded] = npmCommand(args);
  const result = await command(executable, forwarded, {
    ...options,
    env: {...(options.env ?? process.env), npm_config_update_notifier: 'false'},
  });
  if (result.status !== 0) {
    throw new Error(`npm ${args[0]} failed (${result.status ?? result.signal}):\n${result.stderr}`);
  }
  return result;
}
