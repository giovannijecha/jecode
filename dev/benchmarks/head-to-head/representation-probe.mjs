// Supplementary, review-derived API check; separate from frozen trial scoring.
import assert from "node:assert/strict";
import { resolve } from "node:path";
import { pathToFileURL } from "node:url";

const { planBuild } = await import(pathToFileURL(resolve(process.argv[2], "src/index.js")));
const results = [];
for (const completed of [undefined, [], ["a"]]) {
  const hidden = Object.freeze(Object.defineProperties(Object.create(null), {
    id: { value: "b" }, deps: { value: Object.freeze(["a"]) }, duration: { value: 3 },
  }));
  const tasks = Object.freeze([Object.freeze({ id: "a", deps: Object.freeze([]), duration: 2 }), hidden]);
  const options = completed === undefined ? {} : { completed: Object.freeze(completed) };
  const cut = completed?.length === 1;
  try {
    assert.deepEqual(planBuild(Object.freeze({ tasks }), Object.freeze(options)), {
      order: cut ? ["b"] : ["a", "b"], waves: cut ? [["b"]] : [["a"], ["b"]],
      totalDuration: cut ? 3 : 5, criticalPathDuration: cut ? 3 : 5,
    });
    results.push({ completed: completed ?? "omitted", passed: true });
  } catch (error) {
    results.push({ completed: completed ?? "omitted", passed: false, error: String(error) });
  }
}
console.log(JSON.stringify({ supplementary: true, origin: "baseline source review", results }, null, 2));
process.exitCode = results.every(row => row.passed) ? 0 : 1;
