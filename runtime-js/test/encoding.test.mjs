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

test('TextEncoderStream matches native chunk boundaries, conversion and surrogate flushing', async () => {
  const context = vm.createContext({TransformStream, tysel: {
    _utf8Encode(text) { return new TextEncoder().encode(text); },
  }});
  vm.runInContext(readFileSync(new URL('../web-api/source/encoding.js', import.meta.url), 'utf8'), context);
  const Custom = vm.runInContext('TextEncoderStream', context);
  const collect = async (C, chunks) => {
    const stream = new C(), output = [];
    const reading = (async () => { for await (const chunk of stream.readable) output.push(Array.from(chunk)); })();
    const writer = stream.writable.getWriter();
    for (const chunk of chunks) await writer.write(chunk);
    await writer.close(); await reading;
    return output;
  };
  const cases = [[], [''], ['\ud83d', '', '\ude00'], ['\ud800'], ['\udc00', '中'], ['\ud800', '\ud800', '\udc00'], [undefined, null, 42, {toString(){return '😀';}}]];
  let seed = 451;
  for (let i=0;i<80;i++) {
    const chunks=[];
    for(let j=0;j<8;j++) { seed=(Math.imul(seed,1664525)+1013904223)>>>0; chunks.push(String.fromCharCode(seed&0xffff)); }
    cases.push(chunks);
  }
  for (const chunks of cases) assert.deepEqual(await collect(Custom,chunks), await collect(TextEncoderStream,chunks));
  // DOMString conversion rejects Symbols; Node's encoder stream currently stringifies them.
  for (const C of [Custom]) {
    const stream = new C(), reader=stream.readable.getReader(), writer=stream.writable.getWriter();
    const read=assert.rejects(reader.read(), {name:'TypeError'});
    await assert.rejects(writer.write(Symbol('invalid')), {name:'TypeError'});
    await read;
  }
});
