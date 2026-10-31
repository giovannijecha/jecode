import {execFileSync, spawn} from 'node:child_process';
import {createHash} from 'node:crypto';
import fs from 'node:fs';
import path from 'node:path';
import {createRequire} from 'node:module';
import {fileURLToPath} from 'node:url';

export const root = fileURLToPath(new URL('../../', import.meta.url));
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
    const child = spawn(executable, args, {
      cwd: root, windowsHide: true, ...options, stdio: ['pipe', 'pipe', 'pipe'],
    });
    let stdout = '', stderr = '';
    const timer = setTimeout(() => child.kill(), options.timeout ?? 120_000);
    child.stdout.setEncoding('utf8').on('data', chunk => { stdout += chunk; });
    child.stderr.setEncoding('utf8').on('data', chunk => { stderr += chunk; });
    child.on('error', error => { clearTimeout(timer); reject(error); });
    child.on('close', (status, signal) => {
      clearTimeout(timer);
      resolve({status, signal, stdout, stderr});
    });
    child.stdin.end(options.input ?? '');
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
