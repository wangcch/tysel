import assert from 'node:assert/strict';
import fs from 'node:fs';
import vm from 'node:vm';
import test from 'node:test';
const context=vm.createContext({URL,TextEncoder,TextDecoder,__tysel_isReadableStream:()=>false});
vm.runInContext(fs.readFileSync(new URL('../web-api/source/http.js',import.meta.url),'utf8'),context);
const CustomResponse=vm.runInContext('Response',context);
test('Response status and null-body rules match native constructors',()=>{
  for(const status of [undefined,null,0,101,199,200,204,205,299,304,599,600,NaN,Infinity,-1,65736,200.9,'201'])for(const body of [null,undefined,'','A',new Uint8Array(0)]) {
    const run=C=>{try{const r=new C(body,{status});return ['ok',r.status,r.ok,r.type];}catch(e){return [e.name];}};
    assert.deepEqual(run(CustomResponse),run(Response),String(status));
  }
});
test('Response redirect and error preserve immutable headers and clone semantics',async()=>{
  for(const status of [undefined,200,301,302,303,304,307,308,65536+302]) {
    const run=C=>{try{const r=C.redirect('https://example.com/a',status);return [r.status,r.headers.get('location'),r.type];}catch(e){return [e.name];}};
    assert.deepEqual(run(CustomResponse),run(Response));
  }
  for(const r of [CustomResponse.error(),CustomResponse.redirect('https://example.com/')]) {
    for(const h of [r.headers,r.clone().headers])for(const method of ['set','append','delete'])assert.throws(()=>h[method]('a','b'),{name:'TypeError'});
    assert.equal(await r.text(),'');assert.equal(await r.text(),'');assert.equal(r.bodyUsed,false);
  }
  assert.equal(CustomResponse.error().clone().status,0);
});

test('Response reads the status dictionary member once',()=>{
  for(const status of [undefined,200,204])for(const body of [null,'body']) {
    const run=C=>{let reads=0;try{const r=new C(body,{get status(){if(++reads>1)throw new Error('read twice');return status;}});return [r.status,reads];}catch(e){return [e.name,reads];}};
    assert.deepEqual(run(CustomResponse),run(Response));
  }
});
