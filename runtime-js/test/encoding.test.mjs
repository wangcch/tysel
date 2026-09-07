import assert from 'node:assert/strict';
import { readFileSync } from 'node:fs';
import vm from 'node:vm';
import test from 'node:test';

const NativeDecoder = globalThis.TextDecoder;
test('incremental UTF-8 agrees with native decoding across malformed and split input', () => {
  const context = vm.createContext({tysel: {
    _utf8Decode(bytes, fatal) { return new NativeDecoder('utf-8', {fatal, ignoreBOM: true}).decode(bytes); },
  }});
  vm.runInContext(readFileSync(new URL('../web-api/source/encoding.js', import.meta.url), 'utf8'), context);
  const custom = vm.runInContext(`(input, widths, fatal) => {
    const decoder = new TextDecoder('utf-8', {fatal});
    const output = []; let offset = 0;
    try {
      for (const width of widths) {
        output.push(decoder.decode(new Uint8Array(input.slice(offset, offset + width)), {stream:true}));
        offset += width;
      }
      output.push(decoder.decode());
      return JSON.stringify(output);
    } catch (error) { return JSON.stringify({output, error:error.name}); }
  }`, context);
  const cases = [[239,187,191,228,184,173,240,159,152,128], [224,128], [237,160,128], [244,144,128,128], [226,130]];
  let seed = 47829;
  for (let i=0;i<300;i++) {
    const bytes=[];
    for(let j=0;j<12;j++) { seed=(Math.imul(seed,1664525)+1013904223)>>>0; bytes.push(seed>>>24); }
    cases.push(bytes);
  }
  for (const input of cases) for (const fatal of [false,true]) for (const chunkSize of [1,2,3,5]) {
    const widths=[];for(let i=0;i<input.length;i+=chunkSize) widths.push(Math.min(chunkSize,input.length-i));
    const decoder=new NativeDecoder('utf-8',{fatal});const output=[];let offset=0, expected;
    try {
      for(const width of widths) {output.push(decoder.decode(new Uint8Array(input.slice(offset,offset+width)),{stream:true}));offset+=width;}
      output.push(decoder.decode());expected=JSON.stringify(output);
    } catch(error) {expected=JSON.stringify({output,error:error.name});}
    assert.equal(custom(input,widths,fatal),expected,JSON.stringify({input,widths,fatal}));
  }
});
