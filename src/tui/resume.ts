import type { SessionCatalogEntry } from "../sessions/store.ts";
import type { Palette } from "../ui/theme.ts";
import type { Picker } from "./picker.ts";
import { heading } from "./picker.ts";

export function resumePicker(
  candidates: readonly SessionCatalogEntry[],
  palette: Palette,
): Picker {
  return {
    title: heading("resume", "saved conversations", palette),
    searchable: true,
    visible: 8,
    options: candidates.map((candidate) => ({
      label: candidate.preview,
      hint: stamp(candidate.updatedAt),
      value: `${candidate.turns} ${candidate.turns === 1 ? "turn" : "turns"}`,
    })),
    index: 0,
  };
}

function stamp(value: string): string {
  const date = new Date(value);
  if (!Number.isFinite(date.getTime())) return value.replace("T", " ").slice(0, 16);
  const two = (part: number): string => String(part).padStart(2, "0");
  return [
    `${date.getFullYear()}-${two(date.getMonth() + 1)}-${two(date.getDate())}`,
    `${two(date.getHours())}:${two(date.getMinutes())}`,
  ].join(" ");
}
