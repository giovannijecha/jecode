import fs from 'node:fs';
import path from 'node:path';
import {fileURLToPath} from 'node:url';
import {
  root, packageName, targets, host, version, revision, sourceClean, digest, regularFile,
  freshDirectory, json, npm, command,
} from './common.mjs';
import {verifyArchive} from './archive.mjs';
import {auditLinux} from './linux-abi.mjs';

export async function stage(binary, destination) {
  const target = targets[host];
  if (!target) throw new Error(`Unsupported build host: ${host}`);
  const releaseVersion = version();
  const executable = path.resolve(binary);
  const bytes = regularFile(executable);
  const check = await command(executable, ['--version']);
  const expected = `jecode ${releaseVersion} (${target.identity}-release, exe fnv1a64:`;
  if (check.status !== 0 || !check.stdout.trim().startsWith(expected)) {
    throw new Error(`Expected the native ${host} release build ${releaseVersion}; got ${check.stdout.trim() || check.stderr.trim()}`);
  }
  if (digest(regularFile(executable)) !== digest(bytes)) throw new Error('The binary changed during staging.');
  const abi = process.platform === 'linux' ? {
    glibcMinimum: target.glibcMinimum,
    abiCheckedFiles: auditLinux(executable, bytes, target.glibcMinimum),
  } : {};
  const output = freshDirectory(destination);
  fs.writeFileSync(path.join(output, target.binary), bytes, {mode: 0o755, flag: 'wx'});
  json(path.join(output, 'manifest.json'), {
    format: 1, version: releaseVersion, revision: revision(), sourceClean: sourceClean(), target: host,
    binary: target.binary, bytes: bytes.length, sha256: digest(bytes),
    ...abi,
  });
  return output;
}

export function collect(artifacts, {local = false} = {}) {
  const releaseVersion = version(), sourceRevision = revision(), payloads = [];
  for (const [id, target] of Object.entries(targets)) {
    const folder = path.join(artifacts, id);
    if (!fs.existsSync(folder)) {
      if (local) continue;
      throw new Error(`Missing native artifact: ${id}`);
    }
    if (!fs.lstatSync(folder).isDirectory()) throw new Error(`Expected an artifact directory: ${id}`);
    const manifest = JSON.parse(regularFile(path.join(folder, 'manifest.json')));
    if (manifest.format !== 1 || manifest.version !== releaseVersion ||
        manifest.revision !== sourceRevision || manifest.target !== id ||
        manifest.binary !== target.binary) {
      throw new Error(`Artifact version, revision or platform mismatch: ${id}`);
    }
    if (!local && manifest.sourceClean !== true) {
      throw new Error(`Release artifact was staged from modified source: ${id}; use a clean checkout.`);
    }
    if (target.glibcMinimum && manifest.glibcMinimum !== target.glibcMinimum) {
      throw new Error(`Artifact glibc baseline mismatch: ${id}`);
    }
    const bytes = regularFile(path.join(folder, target.binary));
    if (bytes.length !== manifest.bytes || digest(bytes) !== manifest.sha256) {
      throw new Error(`Artifact checksum mismatch: ${id}`);
    }
    payloads.push({id, target, manifest, bytes});
  }
  if (!payloads.length) throw new Error('No native artifacts supplied.');
  return {releaseVersion, sourceRevision, payloads};
}

export async function pack(artifacts, destination, {local = false} = {}) {
  if (!local && !sourceClean()) throw new Error('Complete releases must be assembled from a clean checkout.');
  const {releaseVersion, sourceRevision, payloads} = collect(path.resolve(artifacts), {local});
  const expected = new Map();
  const manifest = {
    name: packageName, version: releaseVersion,
    description: 'A terminal coding agent in Rust',
    license: 'MIT', author: 'Giovanni Jecha',
    repository: {type: 'git', url: 'git+https://github.com/giovannijecha/jecode.git'},
    homepage: 'https://github.com/giovannijecha/jecode#readme',
    bugs: {url: 'https://github.com/giovannijecha/jecode/issues'},
    bin: {jecode: 'bin/jecode.cjs'},
    engines: {node: '>=22'},
    os: ['win32', 'linux', 'darwin'], cpu: ['x64', 'arm64'],
    files: ['bin/', 'native/'],
    publishConfig: {access: 'public', registry: 'https://registry.npmjs.org/'},
    ...(local ? {private: true} : {}),
  };
  expected.set('package.json', Buffer.from(`${JSON.stringify(manifest, null, 2)}\n`));
  expected.set('README.md', regularFile(path.join(root, 'npm/README.md')));
  expected.set('LICENSE', regularFile(path.join(root, 'LICENSE')));
  for (const file of ['jecode.cjs', 'targets.cjs']) {
    // The executable shebang must also work when assembling on a CRLF checkout.
    const source = regularFile(path.join(root, 'npm', file)).toString('utf8').replaceAll('\r\n', '\n');
    expected.set(`bin/${file}`, Buffer.from(source));
  }
  for (const {id, target, bytes} of payloads) expected.set(`native/${id}/${target.binary}`, bytes);
  expected.set('native/manifest.json', Buffer.from(`${JSON.stringify({
    format: 1, version: releaseVersion, revision: sourceRevision,
    artifacts: payloads.map(({manifest: item}) => item),
  }, null, 2)}\n`));

  const output = freshDirectory(destination), packageRoot = path.join(output, 'package');
  fs.mkdirSync(packageRoot);
  for (const [name, bytes] of expected) {
    const file = path.join(packageRoot, name);
    fs.mkdirSync(path.dirname(file), {recursive: true});
    fs.writeFileSync(file, bytes, {flag: 'wx', mode: name.endsWith('.md') || name.endsWith('.json') || name === 'LICENSE' ? 0o644 : 0o755});
  }
  const result = await npm(['pack', '--ignore-scripts', '--json', '--offline',
    '--pack-destination', output, packageRoot], {env: {
      ...process.env, npm_config_cache: path.join(root, '.tmp/npm-cache'), npm_config_logs_max: '0',
    }});
  const info = JSON.parse(result.stdout);
  const filename = `${packageName.slice(1).replace('/', '-')}-${releaseVersion}.tgz`;
  if (info.length !== 1 || info[0].filename !== filename) throw new Error('Unexpected npm pack output.');
  const tarball = path.join(output, filename), bytes = regularFile(tarball);
  verifyArchive(bytes, expected);
  json(path.join(output, 'manifest.json'), {
    format: 1, version: releaseVersion, revision: sourceRevision,
    targets: payloads.map(({id}) => id), publishable: !local,
    filename, bytes: bytes.length, sha256: digest(bytes), integrity: info[0].integrity,
  });
  return tarball;
}

async function main(args) {
  const [action, input, output, option, ...rest] = args;
  if (!['stage', 'pack'].includes(action) || !input || !output || rest.length ||
      (option && (action !== 'pack' || option !== '--local'))) {
    throw new Error('Usage: node tools/npm/release.mjs stage <release-binary> <artifact-dir>\n       node tools/npm/release.mjs pack <artifacts-root> <output-dir> [--local]');
  }
  const result = action === 'stage' ? await stage(input, output) : await pack(input, output, {local: option === '--local'});
  process.stdout.write(`${result}\n`);
}

if (process.argv[1] && path.resolve(process.argv[1]) === fileURLToPath(import.meta.url)) {
  main(process.argv.slice(2)).catch(error => { console.error(error.message); process.exitCode = 1; });
}
