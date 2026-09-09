import assert from 'node:assert/strict';
import {readFileSync} from 'node:fs';
import vm from 'node:vm';
import test from 'node:test';

const context=vm.createContext({TextEncoder,TextDecoder});
vm.runInContext(readFileSync(new URL('../web-api/source/url.js',import.meta.url),'utf8'),context);
const CustomURL=vm.runInContext('URL',context);
const CustomParams=vm.runInContext('URLSearchParams',context);
function state(url) { return [url.href,url.origin,url.host,url.hostname,url.port,url.pathname,url.search,String(url.searchParams)]; }

test('authority paths preserve empty segments through parsing, resolution and setters',()=>{
  const segments=['a','','.','..','%2e','.%2E','%2e%2e'];
  for(const a of segments) for(const b of segments) for(const c of segments) {
    const path='/'+[a,b,c].join('/');
    for(const reference of ['https://example.com'+path,'.'+path]) {
      const base='https://example.com/a//b/';
      assert.deepEqual(state(new CustomURL(reference,base)),state(new URL(reference,base)),reference);
    }
    const custom=new CustomURL('https://example.com/'), native=new URL('https://example.com/');
    custom.pathname=path;native.pathname=path;
    assert.deepEqual(state(custom),state(native),path);
  }
});

test('default ports normalize consistently across URL mutations',()=>{
  for(const [scheme,port] of [['http',80],['https',443],['ws',80],['wss',443],['ftp',21]]) {
    for(const host of ['example.com','[::1]']) {
      const input=`${scheme}://${host}:00${port}/a//b`;
      const custom=new CustomURL(input),native=new URL(input);
      assert.deepEqual(state(custom),state(native));
      for(const [key,value] of [['port','8080'],['port','00'+port],['host',host+':'+port],['href',input]]) {
        custom[key]=value;native[key]=value;assert.deepEqual(state(custom),state(native),key);
      }
    }
  }
  const custom=new CustomURL('http://example.com:443/a'),native=new URL('http://example.com:443/a');
  custom.protocol='https';native.protocol='https';assert.deepEqual(state(custom),state(native));
});

test('form-query decoding replaces invalid bytes and preserves valid neighbors and BOM',()=>{
  const cases=['q=%E4%B8%AD+%FF','q=%EF%BB%BF','q=中+%FF','q=%E2%82','q=%GG+%41%','q=%00','q=\ud800','%FF=a+b'];
  for(let i=0;i<256;i++)cases.push('q=%E4%B8%AD+%'+i.toString(16).padStart(2,'0')+'%41');
  for(const query of cases) {
    // Node 22's raw-Unicode malformed-percent path differs from the byte
    // parser. Percent-encode non-ASCII scalars for the differential reference.
    const reference=Array.from(query).map(c=>c.charCodeAt(0)>127 ? encodeURIComponent(c.toWellFormed()) : c).join('');
    const custom=new CustomParams(query),native=new URLSearchParams(reference);
    assert.equal(JSON.stringify([...custom]),JSON.stringify([...native]),query);
    assert.equal(String(custom),String(native),query);
    const u=new CustomURL('https://example.com/?'+query),n=new URL('https://example.com/?'+reference);
    const retained=u.searchParams;
    u.search='?'+query;n.search='?'+reference;
    u.searchParams.append('x','\ud800😀');n.searchParams.append('x','\ud800😀');
    assert.equal(u.search,n.search);assert.equal(u.searchParams,retained);
  }
  assert.equal(new CustomParams('q=中+%FF').get('q'),'中 �');
  for(const init of [{x:'\ud800'},[['\ud800','\udfff']]])assert.equal(String(new CustomParams(init)),String(new URLSearchParams(init)));
});
