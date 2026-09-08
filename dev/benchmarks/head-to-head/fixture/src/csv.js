// Parse the original unquoted CSV format.
export function parseCsv(text) {
  return text.trim().split(/\r?\n/).map(line => line.split(','));
}
