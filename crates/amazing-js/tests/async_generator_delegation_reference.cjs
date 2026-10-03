// Read-only Node reference diagnostic; not copied into engine code.
(async () => {
  const results = {};
  {
    const log = [], value = { get then() { log.push('get then'); } };
    const iterable = { [Symbol.asyncIterator]() { return { next() { return {done:false,value:0}; }, get return() { log.push('get return'); } }; } };
    async function* f() { yield* iterable; }
    const g=f(); await g.next(); const result=await g.return(value);
    results.missing_return={log,done:result.done,value_identity:result.value===value};
  }
  {
    const log = [], value = { get then() { log.push('output then'); } };
    const iterable = { [Symbol.asyncIterator]() { return { next() { return {done:false,value:0}; }, return() { log.push('return'); return {done:true,value}; } }; } };
    async function* f() { yield* iterable; }
    const g=f(); await g.next(); const result=await g.return(7);
    results.done_return={log,done:result.done,value_identity:result.value===value};
  }
  {
    const log = [], value = { get then() { log.push('output then'); } };
    const iterable = { [Symbol.asyncIterator]() { return {next() {return {done:false,value};}}; } };
    async function* f(){yield* iterable;}
    const result=await f().next();
    results.raw_yield={log,done:result.done,value_identity:result.value===value};
  }
  {
    const log = [], value = { get then() { log.push('done then'); } }; let seen;
    const iterable = { [Symbol.asyncIterator]() { return {next() {return {done:true,value};}}; } };
    async function* f(){seen=yield* iterable;return 0;}
    const result=await f().next();
    results.done_next={log,done:result.done,inner_value_identity:seen===value,result_value:result.value};
  }
  console.log(JSON.stringify({node:process.version,v8:process.versions.v8,results},null,2));
})().catch(e=>{console.error(e);process.exitCode=1});
