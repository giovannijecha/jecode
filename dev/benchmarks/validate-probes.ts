// Calibrate actual probe entry points: less work must not look like an improvement.
import assert from "node:assert/strict";
import { mkdtemp, rm } from "node:fs/promises";
import { tmpdir } from "node:os";
import { join } from "node:path";
import { fileURLToPath } from "node:url";
import { capture, probeEnvironment } from "./capture.ts";

const root = fileURLToPath(new URL("../../", import.meta.url));
const temporary = await mkdtemp(join(tmpdir(), "jecode-probe-calibration-"));
const mutations = [
  { probe: "session", name: "empty catalogue", setup: `
    const {DurableSessionStore} = await import('./src/sessions/store.ts');
    DurableSessionStore.prototype.list = async () => [];` },
  { probe: "session", name: "missing load", setup: `
    const {DurableSessionStore} = await import('./src/sessions/store.ts');
    DurableSessionStore.prototype.load = async () => undefined;` },
  { probe: "session", name: "discarded checkpoint", setup: `
    const {SessionPersistence} = await import('./src/sessions/runtime.ts');
    SessionPersistence.prototype.checkpoint = async () => {};` },
  { probe: "search", name: "always no matches", setup: `
    const {searchText} = await import('./src/tools/search.ts');
    searchText.run = async () => ({output: '[no matches]'});` },
  { probe: "redaction", name: "no redaction", module: "/src/credential-safety.ts",
    source: `export const MAX_REDACTION_SECRETS = 64;
      export const credentialRedactor = () => ({write: text => text, end: () => ''});` },
  { probe: "transcript", name: "empty viewport", module: "/src/tui/transcript-view.ts",
    source: `export const transcriptRenderer = () => ({invalidate() {},
      viewport: () => ({rows: [], pending: false, maxScroll: 0, animating: false})});` },
] as const;

try {
  const env = probeEnvironment(join(temporary, "home"));
  for (const probe of [...new Set(mutations.map(mutation => mutation.probe))]) {
    const result = await capture([`dev/benchmarks/${probe}.ts`], root, env, 120_000);
    assert.equal(result.failure, null, `${probe} reference acquisition`);
    assert.ok(result.stdout.trim(), `${probe} reference failed: ${result.stderr.slice(0, 4_000)}`);
    const report = JSON.parse(result.stdout);
    // Timing thresholds are diagnostic; a correctness failure cannot emit a complete report.
    assert.ok(result.exitCode === 0 || result.exitCode === 1 && report.results.passed === false);
    assert.equal(result.stderr, "", `${probe} reference must pass correctness checks`);
    console.log(`${probe}: reference correctness passed`);
  }
  for (const mutation of mutations) {
    const setup = "setup" in mutation ? mutation.setup : `
      const {registerHooks} = await import('node:module');
      registerHooks({load(url, context, next) {
        if (url.endsWith(${JSON.stringify(mutation.module)}))
          return {format: 'module', source: ${JSON.stringify(mutation.source)}, shortCircuit: true};
        return next(url, context);
      }});`;
    const result = await capture(["--input-type=module", "--eval",
      `${setup}\nawait import('./dev/benchmarks/${mutation.probe}.ts');`], root, env, 120_000);
    assert.equal(result.failure, null, `${mutation.name}: acquisition must succeed`);
    assert.notEqual(result.exitCode, 0, `${mutation.name}: the probe must reject incomplete work`);
    assert.match(result.stderr, /AssertionError/, `${mutation.name}: must fail a correctness assertion`);
    console.log(`${mutation.probe}: rejected ${mutation.name}`);
  }
} finally {
  await rm(temporary, { recursive: true, force: true });
}
