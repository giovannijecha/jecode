// Fixed acceptance checks live outside the agent workspace and never change during a comparison.
import assert from 'node:assert/strict';
import { mkdtempSync, writeFileSync, rmSync } from 'node:fs';
import { tmpdir } from 'node:os';
import { resolve, join } from 'node:path';
import { pathToFileURL } from 'node:url';
import { spawnSync } from 'node:child_process';

const root = resolve(process.argv[2]);
const { summarizeLedger } = await import(pathToFileURL(join(root, 'src/ledger.js')).href);
const results = [];
const header = 'date,account,amount';
const csv = rows => `${header}\n${rows.join('\n')}\n`;
const empty = { entries: 0, totalCents: 0, accounts: [] };
const one = (account, cents) => ({ entries: 1, totalCents: cents,
  accounts: [{ account, entries: 1, totalCents: cents }] });
function check(name, run) {
  try { run(); results.push({ name, passed: true }); }
  catch (error) { results.push({ name, passed: false, error: String(error.message).slice(0,1500) }); }
}
function reject(name, text, filters) {
  check(name, () => assert.throws(() => summarizeLedger(text, filters)));
}
check('original behavior', () => assert.deepEqual(summarizeLedger(csv(['2026-01-01,Sales,12.50'])), one('Sales',1250)));
check('header only', () => assert.deepEqual(summarizeLedger(header), empty));
check('header with final CRLF', () => assert.deepEqual(summarizeLedger(header+'\r\n'), empty));
check('BOM and CRLF', () => assert.deepEqual(summarizeLedger('\ufeff'+header+'\r\n2024-02-29,A,1.2\r\n'),one('A',120)));
check('quoted comma', () => assert.deepEqual(summarizeLedger(csv(['2026-01-01,"A,B",1'])),one('A,B',100)));
check('escaped quotes', () => assert.deepEqual(summarizeLedger(csv(['2026-01-01,"A ""B""",1'])),one('A "B"',100)));
check('quoted newline', () => assert.deepEqual(summarizeLedger(csv(['2026-01-01,"A\nB",1'])),one('A\nB',100)));
check('trim decoded fields', () => assert.deepEqual(summarizeLedger(csv([' 2026-01-01 , A , -1.20 '])),one('A',-120)));
check('exact negative money', () => assert.deepEqual(summarizeLedger(csv(['2026-01-01,A,-0.29'])),one('A',-29)));
check('normalize negative zero', () => assert.deepEqual(summarizeLedger(csv(['2026-01-01,A,-0.00'])),one('A',0)));
check('safe integer boundary', () => assert.deepEqual(summarizeLedger(csv(['2026-01-01,A,90071992547409.91'])),one('A',Number.MAX_SAFE_INTEGER)));
check('year below 100', () => assert.deepEqual(summarizeLedger(csv(['0096-02-29,A,1'])),one('A',100)));
check('string ordering and aggregation', () => assert.deepEqual(summarizeLedger(csv([
  '2026-01-01,a,1.01','2026-01-01,Z,2','2026-01-02,a,-0.01',
])),{entries:3,totalCents:300,accounts:[{account:'Z',entries:1,totalCents:200},{account:'a',entries:2,totalCents:100}]}));
const input = csv(['2026-01-01,A,1','2026-01-02,B,2','2026-01-03,A,3']);
check('inclusive filters', () => assert.deepEqual(summarizeLedger(input,{from:'2026-01-02',to:'2026-01-03',account:' A '}),one('A',300)));
check('empty filtered result', () => assert.deepEqual(summarizeLedger(input,{account:'a'}),empty));
check('input is not mutated or stateful', () => {
  const filters=Object.freeze({account:'A'});
  const first=summarizeLedger(input,filters);
  assert.deepEqual(summarizeLedger(input,filters),first);
});
for(const [name,text] of [
  ['empty input',''],['wrong header','date,account,value\n2026-01-01,A,1'],
  ['extra column',csv(['2026-01-01,A,1,x'])],['missing column',csv(['2026-01-01,A'])],
  ['blank record',input+'\n'],['empty account',csv(['2026-01-01, ,1'])],
  ['unclosed quote',csv(['2026-01-01,"A,1'])],['quote in unquoted field',csv(['2026-01-01,A"B,1'])],
  ['junk after quote',csv(['2026-01-01,"A"x,1'])],
]) reject(name,text);
for(const date of ['2025-02-29','2026-04-31','2026-13-01','0000-01-01','2026-1-01'])
  reject('invalid date '+date,csv([`${date},A,1`]));
for(const amount of ['+1','1e2','NaN','Infinity','.5','1.','1.001','90071992547409.92'])
  reject('invalid amount '+amount,csv([`2026-01-01,A,${amount}`]));
reject('unsafe running total',csv(['2026-01-01,A,90071992547409.91','2026-01-01,B,0.01']));
reject('unsafe account total',csv(['2026-01-01,A,90071992547409.91','2026-01-01,B,-90071992547409.91','2026-01-01,A,0.01']));
reject('invalid excluded row',csv(['2026-01-01,B,invalid']),{account:'A'});
for(const filters of [{from:'bad'},{to:'2026-02-30'},{from:'2026-02-01',to:'2026-01-01'},{account:' '},{unknown:'x'}])
  reject('invalid filter '+JSON.stringify(filters),input,filters);

const temp = mkdtempSync(join(tmpdir(),'ledger-acceptance-'));
try {
  const file=join(temp,'input.csv'); writeFileSync(file,input);
  const bad=join(temp,'bad.csv'); writeFileSync(bad,'invalid');
  function cli(args) {
    const result=spawnSync(process.execPath,[join(root,'src/cli.js'),...args],{cwd:root,encoding:'utf8',timeout:5000,maxBuffer:1024*1024});
    if(result.error) throw result.error;
    return result;
  }
  for(const args of [[file,'--account','A','--from','2026-01-03'],['--from','2026-01-03',file,'--account','A']])
    check('CLI filters '+args.indexOf(file),()=>{
      const result=cli(args); assert.equal(result.status,0,result.stderr);
      assert.equal(result.stderr,''); assert.ok(result.stdout.endsWith('\n'));
      assert.deepEqual(JSON.parse(result.stdout),one('A',300));
    });
  check('CLI help',()=>{const result=cli(['--help']); assert.equal(result.status,0); assert.match(result.stdout,/usage/i); assert.equal(result.stderr,'');});
  for(const [name,args] of [
    ['missing file',[]],['unknown flag',[file,'--bogus']],['extra file',[file,file]],
    ['missing option value',[file,'--from']],['duplicate option',[file,'--account','A','--account','A']],
    ['unreadable file',[join(temp,'absent.csv')]],['invalid file',[bad]],
  ]) check('CLI rejects '+name,()=>{
    const result=cli(args); assert.equal(result.status,1); assert.equal(result.stdout,'');
    assert.ok(result.stderr.trim()); assert.doesNotMatch(result.stderr,/\n\s+at /);
  });
} finally { rmSync(temp,{recursive:true,force:true}); }
console.log(JSON.stringify({total:results.length,passed:results.filter(r=>r.passed).length,results},null,2));
process.exitCode=results.every(r=>r.passed)?0:1;
