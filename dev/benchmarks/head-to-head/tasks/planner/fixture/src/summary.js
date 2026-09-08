export function summarize(graph, { order, waves }) {
  const totalDuration = order.reduce((sum, id) => sum + graph.get(id).duration, 0);
  return { order, waves, totalDuration, criticalPathDuration: totalDuration };
}
