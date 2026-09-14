import assert from 'node:assert/strict';
import { readFileSync } from 'node:fs';
import test from 'node:test';
import vm from 'node:vm';

function fixture() {
  const state = { now: 0, jobs: [], waits: [], canceled: [] };
  const context = vm.createContext({ tysel: {
    _timerNow: () => state.now,
    _queueMicrotask: (fn) => state.jobs.push(fn),
    _sleepOp: (delay) => {
      const id = state.waits.length + 1;
      let resolve;
      const promise = new Promise((done) => { resolve = done; });
      state.waits.push({ delay, resolve });
      return { id, promise };
    },
    _cancelOp: (id) => state.canceled.push(id),
  }});
  vm.runInContext(readFileSync(new URL('../web-api/source/timers.js', import.meta.url), 'utf8'), context);
  return { state, context };
}

test('deferred timer admission preserves its original deadline', async () => {
  const { state, context } = fixture();
  let calls = 0;
  context.setTimeout(() => calls++, 50);
  state.now = 75;
  state.jobs.shift()();
  assert.equal(state.waits[0].delay, 0);
  state.waits[0].resolve();
  await new Promise(setImmediate);
  assert.equal(calls, 1);
});

test('interval callback time counts toward the next deadline and clear cancels the wait', async () => {
  const { state, context } = fixture();
  let calls = 0;
  const id = context.setInterval(() => { calls++; state.now += 30; }, 20);
  state.jobs.shift()();
  assert.equal(state.waits[0].delay, 20);
  state.now = 20;
  state.waits[0].resolve();
  await new Promise(setImmediate);
  state.jobs.shift()();
  assert.equal(state.waits[1].delay, 0);
  context.clearInterval(id);
  assert.deepEqual(state.canceled, [2]);
  state.waits[1].resolve();
  await new Promise(setImmediate);
  assert.equal(calls, 1);
});
