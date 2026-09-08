// Expectations are independent examples, fixed before any live model output.
import assert from 'node:assert/strict';
import { resolve, join } from 'node:path';
import { pathToFileURL } from 'node:url';
import { mkdtempSync, writeFileSync, rmSync } from 'node:fs';
import { tmpdir } from 'node:os';
import { spawnSync } from 'node:child_process';

const root=resolve(process.argv[2]);
const { planBuild }=await import(pathToFileURL(join(root,'src/index.js')));
const results=[];
const task=(id,deps=[],duration=1) => ({id,deps,duration});
const doc=(...tasks) => ({tasks});
function check(name,run) {
  try { run(); results.push({name,passed:true}); }
  catch(error) { results.push({name,passed:false,error:String(error.message).slice(0,500)}); }
}
check('empty graph',()=>assert.deepEqual(planBuild(doc()),{order:[],waves:[],totalDuration:0,criticalPathDuration:0}));
check('available tasks compete immediately',()=>assert.deepEqual(planBuild(doc(
  task('z'),task('b'),task('a',['b']),task('c',['a'],3),
)),{order:['b','a','c','z'],waves:[['b','z'],['a'],['c']],totalDuration:6,criticalPathDuration:5}));
check('diamond dependencies counted once',()=>assert.deepEqual(planBuild(doc(
  task('d',['b','c'],4),task('c',['a'],3),task('b',['a'],2),task('a',[],1),
)),{order:['a','b','c','d'],waves:[['a'],['b','c'],['d']],totalDuration:10,criticalPathDuration:8}));
check('exact string order including prototype names',()=>{
  const ids=['a','Z','__proto__','constructor','A','0'];
  assert.deepEqual(planBuild(doc(...ids.map(id=>task(id)))).order,[...ids].sort());
});
const graph=doc(task('root',[],2),task('left',['root'],3),task('right',['root'],5),task('unused',[],100));
const selected={order:['root','left','right'],waves:[['root'],['left','right']],totalDuration:10,criticalPathDuration:7};
check('target closure deduplicates shared dependencies',()=>assert.deepEqual(planBuild(graph,{targets:['right','left']}),selected));
check('single target closure',()=>assert.deepEqual(planBuild(graph,{targets:['left']}),{
  order:['root','left'],waves:[['root'],['left']],totalDuration:5,criticalPathDuration:5,
}));
check('frozen inputs are unchanged',()=>{
  const document=structuredClone(graph), options={targets:['right','left']};
  document.tasks.forEach(t=>{Object.freeze(t.deps);Object.freeze(t);});
  Object.freeze(document.tasks);Object.freeze(document);Object.freeze(options.targets);Object.freeze(options);
  assert.deepEqual(planBuild(document,options),selected);
  assert.deepEqual(document,graph);
});
check('zero and maximum safe duration',()=>assert.deepEqual(planBuild(doc(task('a',[],0),task('b',['a'],Number.MAX_SAFE_INTEGER))),{
  order:['a','b'],waves:[['a'],['b']],totalDuration:Number.MAX_SAFE_INTEGER,criticalPathDuration:Number.MAX_SAFE_INTEGER,
}));
check('exclude overflow from unselected component',()=>assert.equal(planBuild(doc(
  task('a',[],Number.MAX_SAFE_INTEGER),task('b',[],Number.MAX_SAFE_INTEGER),task('x',[],2),
),{targets:['x']}).totalDuration,2));
check('included sum rejects overflow',()=>assert.throws(()=>planBuild(doc(task('a',[],Number.MAX_SAFE_INTEGER),task('b',[],1)))));
check('deep iterative graph',()=>{
  const count=14000;
  const tasks=Array.from({length:count},(_,i)=>task(`n${i}`,i?[`n${i-1}`]:[],1));
  const value=planBuild({tasks:tasks.reverse()},{targets:[`n${count-1}`]});
  assert.equal(value.order.length,count);assert.equal(value.waves.length,count);
  assert.equal(value.order[0],'n0');assert.equal(value.order.at(-1),`n${count-1}`);
  assert.equal(value.criticalPathDuration,count);assert.equal(value.totalDuration,count);
});
for(const [name,document] of [
  ['null',null],['array',[]],['missing tasks',{}],['wrong tasks',{tasks:{}}],
  ['extra document field',{tasks:[],extra:true}],['null task',doc(null)],
  ['missing duration',{tasks:[{id:'a',deps:[]}]}],['extra task field',doc({...task('a'),command:'danger'})],
  ['empty id',doc(task(''))],['invalid id',doc(task('a.b'))],['numeric id',doc(task(1))],
  ['duplicate ids',doc(task('a'),task('a'))],['unknown dependency',doc(task('a',['b']))],
  ['non-array deps',doc(task('a',null))],['duplicate deps',doc(task('a'),task('b',['a','a']))],
  ['self cycle',doc(task('a',['a']))],['two node cycle',doc(task('a',['b']),task('b',['a']))],
  ...[-1,NaN,Infinity,1.5,'2',null,Number.MAX_SAFE_INTEGER+1].map(duration=>[
    `invalid duration ${String(duration)}`,doc(task('a',[],duration)),
  ]),
]) check(`reject ${name}`,()=>assert.throws(()=>planBuild(document)));
for(const options of [null,[],{other:1},{targets:[]},{targets:'a'},{targets:['missing']},{targets:['a','a']},{targets:[1]}])
  check(`reject options ${JSON.stringify(options)}`,()=>assert.throws(()=>planBuild(doc(task('a')),options)));
for(const invalid of [doc(task('ok'),task('a',['b']),task('b',['a'])),doc(task('ok'),task('a',['missing']))])
  check(`invalid excluded component ${invalid.tasks.length}`,()=>assert.throws(()=>planBuild(invalid,{targets:['ok']})));
const temporary=mkdtempSync(join(tmpdir(),'plan-acceptance-'));
try {
  const file=join(temporary,'input.json'), bad=join(temporary,'invalid.json');
  writeFileSync(file,JSON.stringify(graph));writeFileSync(bad,'{');
  function cli(args) {
    const result=spawnSync(process.execPath,[join(root,'src/cli.js'),...args],{cwd:root,encoding:'utf8',timeout:3000,maxBuffer:1024*1024});
    if(result.error) throw result.error;
    return result;
  }
  for(const args of [[file,'--target','left','--target','right'],['--target','right',file,'--target','left']])
    check(`CLI selection file at ${args.indexOf(file)}`,()=>{
      const result=cli(args);assert.equal(result.status,0,result.stderr);assert.equal(result.stderr,'');
      assert.ok(result.stdout.endsWith('\n'));assert.deepEqual(JSON.parse(result.stdout),selected);
    });
  check('CLI help',()=>{const result=cli(['--help']);assert.equal(result.status,0);assert.match(result.stdout,/usage/i);assert.equal(result.stderr,'');});
  for(const [name,args] of [
    ['missing file',[]],['unknown flag',[file,'--no']],['missing value',[file,'--target']],
    ['duplicate target',[file,'--target','left','--target','left']],['extra file',[file,file]],
    ['combined help',[file,'--help']],['unreadable',[join(temporary,'missing')]],['bad JSON',[bad]],
    ['unknown target',[file,'--target','missing']],
  ]) check(`CLI rejects ${name}`,()=>{
    const result=cli(args);assert.equal(result.status,1);assert.equal(result.stdout,'');
    assert.ok(result.stderr.trim());assert.doesNotMatch(result.stderr,/\n\s+at /);
  });
} finally { rmSync(temporary,{recursive:true,force:true}); }
console.log(JSON.stringify({total:results.length,passed:results.filter(r=>r.passed).length,results},null,2));
process.exitCode=results.every(r=>r.passed)?0:1;
