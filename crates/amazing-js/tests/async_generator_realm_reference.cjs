// Reference-only diagnostic: Node/V8 is executed on Boat, never linked or ported.
const vm = require('node:vm');
const child = vm.createContext({});
const get = source => vm.runInContext(source, child);
const C = {array:get('Array.prototype'),object:get('Object.prototype'),promise:get('Promise.prototype')};
const P = {array:Array.prototype,object:Object.prototype,promise:Promise.prototype};
const proto = Object.getPrototypeOf(Object.getPrototypeOf((async function*(){})()));
const realm = (value,kind) => Object.getPrototypeOf(value)===C[kind]?'child':Object.getPrototypeOf(value)===P[kind]?'parent':'other';
async function main(){
  const rows=[];
  for(const [name,body] of [
    ['literal-yield','yield [];return []'],
    ['explicit-await-before-yield','await 0;yield [];return []'],
    ['thenable-before-yield','await {then(r){r(0)}};yield [];return []'],
  ]){
    const it=get(`(async function*(){${body}})`)();
    const request=proto.next.call(it);
    const first=await request;
    const second=await proto.next.call(it);
    const completed=await proto.next.call(it);
    rows.push({name,request:realm(request,'promise'),yieldResult:realm(first,'object'),yieldValue:realm(first.value,'array'),returnResult:realm(second,'object'),returnValue:realm(second.value,'array'),completedResult:realm(completed,'object')});
  }
  for(const [name,body] of [['never-started','yield []'],['already-completed','return []']]){
    const it=get(`(async function*(){${body}})`)();
    if(name==='already-completed')await proto.next.call(it);
    const request=proto.return.call(it,7);const result=await request;
    rows.push({name,request:realm(request,'promise'),returnResult:realm(result,'object'),value:result.value});
  }
  {
    const it=get('(async function*(){try{yield []}finally{yield []}})')();
    await proto.next.call(it);
    const request=proto.return.call(it,Promise.resolve(7));const result=await request;
    const done=await proto.next.call(it);
    rows.push({name:'injected-return-finally-yield',request:realm(request,'promise'),yieldResult:realm(result,'object'),yieldValue:realm(result.value,'array'),returnResult:realm(done,'object'),value:done.value});
  }
  console.log(JSON.stringify({engine:process.versions,rows},null,2));
}
main().catch(error=>{console.error(error);process.exitCode=1});
