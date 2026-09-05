// Shared line and entry counts in filesystem tool summaries.

export function count(text: string, noun: string): string {
  if (text === "") return "empty";
  return plural(text.split("\n").length, noun, `${noun}s`);
}

export function plural(n: number, one: string, many: string): string {
  return `${n} ${n === 1 ? one : many}`;
}
