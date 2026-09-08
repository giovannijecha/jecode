// Supplementary, post-hoc quality probe. This never replaces the frozen scorer.
// Generate the expected ledger from source rows, independently of CSV parsing.
import assert from 'node:assert/strict';
import { createHash } from 'node:crypto';
import { readFileSync } from 'node:fs';
import { resolve, join } from 'node:path';
import { pathToFileURL } from 'node:url';

const root = resolve(process.argv[2]);
const { summarizeLedger } = await import(pathToFileURL(join(root, 'src/ledger.js')).href);
const seed = 0x4c454447;
let state = seed;
function random(n) {
  state ^= state << 13; state ^= state >>> 17; state ^= state << 5;
  return (state >>> 0) % n;
}
const results = [];
const names = ['A', 'a', 'Z', 'A,B', 'say "hello"', 'line\nbreak', 'line\r\nbreak',
  '__proto__', 'constructor', '東京', '🦎', 'é', 'e\u0301', '  trim me  '];
const dates = ['0001-01-01', '0096-02-29', '1900-03-01', '2000-02-29', '2026-12-31', '9999-12-31'];
const quote = value => `"${value.replaceAll('"', '""')}"`;
const amount = cents => `${cents < 0 ? '-' : ''}${Math.floor(Math.abs(cents)/100)}.${String(Math.abs(cents)%100).padStart(2,'0')}`;
const encode = (rows, newline) => 'date,account,amount' + newline +
  rows.map(row => [row.date, quote(row.account), amount(row.cents)].join(',')).join(newline);

function check(name, run) {
  try { run(); results.push({name, passed:true}); }
  catch(error) { results.push({name, passed:false, error:String(error.message).slice(0,1500)}); }
}

function expected(rows, filters) {
  const included = rows.filter(row => (!filters.from || row.date >= filters.from) &&
    (!filters.to || row.date <= filters.to) && (!filters.account || row.account.trim() === filters.account.trim()));
  const accounts = [...new Set(included.map(row => row.account.trim()))].sort().map(account => {
    const members = included.filter(row => row.account.trim() === account);
    return {account, entries:members.length, totalCents:Number(members.reduce((sum,row) => sum+BigInt(row.cents),0n))};
  });
  return {entries:included.length, totalCents:Number(included.reduce((sum,row) => sum+BigInt(row.cents),0n)), accounts};
}

for(let index=0;index<400;index++) {
  const rows = Array.from({length:1+random(12)}, () => ({
    date:dates[random(dates.length)], account:names[random(names.length)], cents:random(200001)-100000,
  }));
  const newline = index%2 ? '\r\n' : '\n';
  const text = (index%3 ? '' : '\ufeff') + encode(rows,newline) + (index%4 ? newline : '');
  const filters = index%5===0 ? {account:names[random(names.length)]} : index%5===1 ? {from:'0096-02-29',to:'2026-12-31'} : {};
  check(`generated CSV ${index}`, () => assert.deepEqual(summarizeLedger(text,filters),expected(rows,filters)));
}

// Literal boundary cases follow the original task, including validation of
// malformed rows excluded by a filter. These do not invent new API contracts.
const header = 'date,account,amount\n';
for(const date of ['0000-02-29','0100-02-29','1900-02-29','2100-02-29','2026-00-01','2026-01-00','2026-01-32','2026-02-29','10000-01-01']) {
  check(`reject calendar ${date}`, () => assert.throws(() => summarizeLedger(`${header}${date},excluded,1`,{account:'included'})));
}
for(const value of ['--1','-','+0','0x10','1,000','１','  ','1.000','900719925474099999999999999999999999999999','-90071992547409.92']) {
  check(`reject amount ${JSON.stringify(value)}`, () => assert.throws(() => summarizeLedger(`${header}2026-01-01,excluded,${quote(value)}`,{account:'included'})));
}
check('negative safe integer boundary', () => assert.deepEqual(
  summarizeLedger(`${header}2026-01-01,A,-90071992547409.91`),
  {entries:1,totalCents:-Number.MAX_SAFE_INTEGER,accounts:[{account:'A',entries:1,totalCents:-Number.MAX_SAFE_INTEGER}]}));
check('negative running sum overflow', () => assert.throws(() =>
  summarizeLedger(`${header}2026-01-01,A,-90071992547409.91\n2026-01-01,B,-0.01`)));
check('overflow before later cancellation', () => assert.throws(() =>
  summarizeLedger(`${header}2026-01-01,A,90071992547409.91\n2026-01-01,B,0.01\n2026-01-01,C,-0.01`)));

console.log(JSON.stringify({kind:'supplementary-post-hoc', seed,
  probeSha256:createHash('sha256').update(readFileSync(new URL(import.meta.url))).digest('hex'),
  total:results.length, passed:results.filter(row=>row.passed).length, results},null,2));
process.exitCode = results.every(row=>row.passed) ? 0 : 1;
