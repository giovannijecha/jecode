import { parseCsv } from './csv.js';

export function summarizeLedger(text, filters = {}) {
  const [, ...rows] = parseCsv(text);
  const accounts = new Map();
  let totalCents = 0;
  for (const [date, account, amount] of rows) {
    const cents = Math.round(Number(amount) * 100);
    totalCents += cents;
    const entry = accounts.get(account) ?? { account, entries: 0, totalCents: 0 };
    entry.entries++;
    entry.totalCents += cents;
    accounts.set(account, entry);
  }
  return { entries: rows.length, totalCents, accounts: [...accounts.values()] };
}
