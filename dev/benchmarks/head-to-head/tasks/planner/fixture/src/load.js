import { readFileSync } from 'node:fs';

export function loadDocument(file) {
  return JSON.parse(readFileSync(file, 'utf8'));
}
