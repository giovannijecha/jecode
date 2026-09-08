export function parsePointer(pointer) {
  if (pointer === '') return [];
  if (typeof pointer !== 'string' || !pointer.startsWith('/')) throw new Error('Invalid pointer');
  return pointer.slice(1).split('/').map(part => part.replaceAll('~1', '/').replaceAll('~0', '~'));
}
export function patchDocument(document, operations) {
  const output = structuredClone(document);
  for (const operation of operations) {
    const parts = parsePointer(operation.path);
    let parent = output;
    for (const part of parts.slice(0, -1)) parent = parent[part];
    if (operation.op !== 'set' || parts.length === 0) throw new Error('Unsupported operation');
    parent[parts.at(-1)] = operation.value;
  }
  return output;
}
