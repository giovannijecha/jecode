import { validateDocument } from './validate.js';
import { buildGraph } from './graph.js';
import { selectTasks } from './select.js';
import { schedule } from './schedule.js';
import { summarize } from './summary.js';

export function planBuild(document, options = {}) {
  const tasks = validateDocument(document);
  const graph = buildGraph(tasks);
  const selected = selectTasks(graph, options);
  const result = schedule(selected);
  return summarize(selected, result);
}
