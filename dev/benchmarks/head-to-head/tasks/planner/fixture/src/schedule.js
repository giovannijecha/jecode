export function schedule(graph) {
  const visited = new Set();
  const order = [];
  function visit(id) {
    if (visited.has(id)) return;
    visited.add(id);
    for (const dependency of graph.get(id).deps) visit(dependency);
    order.push(id);
  }
  for (const id of graph.keys()) visit(id);
  return { order, waves: [order] };
}
