(() => {
  const timers = new Map();
  let nextTimerId = 1;
  let timerGeneration = 0;

  function scheduleTimer(fn, ms, interval, args) {
    if (typeof fn !== "function") {
      throw new TypeError("timer callback must be a function");
    }
    const id = nextTimerId++;
    const delay = Math.max(0, Number(ms) || 0);
    const generation = timerGeneration;
    let cleared = false;
    timers.set(id, () => {
      cleared = true;
    });
    const tick = async () => {
      try {
        await tysel.sleep(delay);
      } catch {
        timers.delete(id);
        return;
      }
      if (cleared || generation !== timerGeneration) {
        timers.delete(id);
        return;
      }
      if (interval) tick();
      else timers.delete(id);
      fn.apply(undefined, args);
    };
    tick();
    return id;
  }

  globalThis.setTimeout = function (fn, ms) {
    return scheduleTimer(fn, ms, false, Array.prototype.slice.call(arguments, 2));
  };
  globalThis.setInterval = function (fn, ms) {
    return scheduleTimer(fn, ms, true, Array.prototype.slice.call(arguments, 2));
  };
  globalThis.clearTimeout = function (id) {
    const clear = timers.get(id);
    if (clear) {
      clear();
      timers.delete(id);
    }
  };
  globalThis.clearInterval = globalThis.clearTimeout;
  globalThis.__tysel_resetTimers = function () {
    timerGeneration++;
    timers.forEach((clear) => clear());
    timers.clear();
  };
})();
