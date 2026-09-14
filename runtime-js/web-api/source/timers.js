(() => {
  const timers = new Map();
  let nextTimerId = 1;
  let timerGeneration = 0;

  function clearTimer(timer) {
    timers.delete(timer.id);
    timer.fn = null;
    timer.args = null;
    if (timer.operation) tysel._cancelOp(timer.operation.id);
    timer.operation = null;
  }

  function tick(timer) {
    if (!timer.fn) return;
    // Admission errors must escape the microtask, not silently drop a timer.
    timer.operation = tysel._sleepOp(Math.max(0, timer.deadline - tysel._timerNow()));
    waitTimer(timer);
  }

  async function waitTimer(timer) {
    try {
      await timer.operation.promise;
    } catch {
      clearTimer(timer);
      return;
    }
    timer.operation = null;
    if (!timer.fn || timer.generation !== timerGeneration) return;
    const fn = timer.fn;
    const args = timer.args;
    if (timer.interval) {
      timer.deadline = tysel._timerNow() + timer.delay;
      tysel._queueMicrotask(() => tick(timer));
    } else clearTimer(timer);
    fn.apply(undefined, args);
  }

  function scheduleTimer(fn, ms, interval, args) {
    if (typeof fn !== "function") {
      throw new TypeError("timer callback must be a function");
    }
    const id = nextTimerId++;
    const delay = Math.max(0, Number(ms) || 0);
    const timer = {id, delay, deadline: tysel._timerNow() + delay, interval, fn, args,
      generation: timerGeneration, operation: null};
    timers.set(id, timer);
    // A timer cleared in this turn needs no native operation. The shared tick
    // function only retains this mutable record, whose callback can be freed
    // immediately even while a started Sleep is being canceled.
    tysel._queueMicrotask(() => tick(timer));
    return id;
  }

  globalThis.setTimeout = function (fn, ms) {
    return scheduleTimer(fn, ms, false, Array.prototype.slice.call(arguments, 2));
  };
  globalThis.setInterval = function (fn, ms) {
    return scheduleTimer(fn, ms, true, Array.prototype.slice.call(arguments, 2));
  };
  globalThis.clearTimeout = function (id) {
    const timer = timers.get(id);
    if (timer) clearTimer(timer);
  };
  globalThis.clearInterval = globalThis.clearTimeout;
  globalThis.__tysel_resetTimers = function () {
    timerGeneration++;
    timers.forEach(clearTimer);
    timers.clear();
  };
})();
