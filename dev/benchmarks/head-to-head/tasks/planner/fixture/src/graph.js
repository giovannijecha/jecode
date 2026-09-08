export function buildGraph(tasks) {
  const graph = new Map();
  for (const task of tasks) graph.set(task.id, task);
  return graph;
}
