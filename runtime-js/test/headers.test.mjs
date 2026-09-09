import assert from 'node:assert/strict';
import fs from 'node:fs';
import vm from 'node:vm';
import test from 'node:test';
const context=vm.createContext({});
vm.runInContext(fs.readFileSync(new URL('../web-api/source/http.js',import.meta.url),'utf8'),context);
const CustomHeaders=vm.runInContext('Headers',context);

test('Headers live iteration follows mutation for entries, keys and values',()=>{
  const cases=[
    [['next'],['next'],['next'],['next'],['next'],['append','z','after end'],['next'],['next']],
    [['next'],['delete','b'],['next'],['next']],
    [['next'],['delete','a'],['next'],['next']],
    [['next'],['set','b','updated'],['next'],['next']],
    [['next'],['append','b','extra'],['next'],['next']],
    [['next'],['set','0','before'],['next'],['next'],['next'],['next']],
    [['next'],['append','z','after'],['next'],['next'],['next'],['next']],
    [['next'],['next'],['set','set-cookie','replacement'],['next'],['next'],['next']],
    [['next'],['next'],['delete','set-cookie'],['next'],['next']],
    [['next'],['next'],['append','set-cookie','third=3'],['next'],['next'],['next'],['next']],
  ];
  for(const method of ['entries','keys','values'])for(const actions of cases) {
    const run=Constructor=>{
      const headers=new Constructor([['a','1'],['b','2'],['set-cookie','first=1'],['set-cookie','second=2']]);
      const iterator=headers[method](),output=[];
      for(const [operation,...args] of actions) {
        if(operation==='next')output.push(iterator.next());else headers[operation](...args);
      }
      return JSON.stringify(output);
    };
    assert.equal(run(CustomHeaders),run(Headers),JSON.stringify({method,actions}));
  }
});

test('Headers forEach and reconstruction do not resurrect deleted headers',()=>{
  const h=new CustomHeaders({a:'1',b:'2',c:'3'}),seen=[];
  h.forEach((value,key)=>{seen.push([key,value]);if(key==='a'){h.delete('b');h.set('c','updated');h.append('d','4');}});
  assert.equal(JSON.stringify(seen),JSON.stringify([['a','1'],['c','updated'],['d','4']]));
  const iterator=h.entries();iterator.next();h.delete('c');
  const copy=new CustomHeaders(iterator);
  assert.equal(copy.has('c'),false);assert.equal(copy.get('d'),'4');
  const first=h.entries().next().value;first[1]='modified';assert.equal(h.get('a'),'1');
});
