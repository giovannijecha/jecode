// Greedy byte-pair merges with ranked candidates and stable leftmost tie breaking.
type Pair = { rank: number; left: number; right: number; end: number };

export function countPiece(bytes: Buffer, ranks: ReadonlyMap<string, number>): number {
  const text = bytes.toString("latin1");
  if (ranks.has(text)) return 1;
  const next = Int32Array.from({ length: bytes.length }, (_, i) => i + 1);
  const previous = Int32Array.from({ length: bytes.length }, (_, i) => i - 1);
  const heap: Pair[] = [];
  let count = bytes.length;
  const offer = (left: number): void => {
    if (left < 0 || left >= bytes.length) return;
    const right = next[left] as number;
    if (right < 0 || right >= bytes.length) return;
    const end = next[right] as number;
    const rank = ranks.get(text.slice(left, end));
    if (rank !== undefined) push(heap, { rank, left, right, end });
  };
  for (let i = 0; i + 1 < bytes.length; i++) offer(i);
  while (heap.length > 0) {
    const pair = pop(heap);
    if (next[pair.left] !== pair.right || next[pair.right] !== pair.end) continue;
    next[pair.left] = pair.end;
    next[pair.right] = -1;
    if (pair.end < bytes.length) previous[pair.end] = pair.left;
    count--;
    offer(previous[pair.left] as number);
    offer(pair.left);
  }
  return count;
}

function before(a: Pair, b: Pair): boolean {
  return a.rank < b.rank || (a.rank === b.rank && a.left < b.left);
}

function push(heap: Pair[], pair: Pair): void {
  let index = heap.length;
  heap.push(pair);
  while (index > 0) {
    const parent = (index - 1) >>> 1;
    if (!before(pair, heap[parent] as Pair)) break;
    heap[index] = heap[parent] as Pair;
    index = parent;
  }
  heap[index] = pair;
}

function pop(heap: Pair[]): Pair {
  const first = heap[0] as Pair;
  const tail = heap.pop() as Pair;
  if (heap.length === 0) return first;
  let index = 0;
  while (index * 2 + 1 < heap.length) {
    let child = index * 2 + 1;
    if (child + 1 < heap.length && before(heap[child + 1] as Pair, heap[child] as Pair)) child++;
    if (!before(heap[child] as Pair, tail)) break;
    heap[index] = heap[child] as Pair;
    index = child;
  }
  heap[index] = tail;
  return first;
}
