(() => {
  const signalToken = Object.freeze({});
  const states = new WeakMap();

  function defaultReason() {
    return new DOMException("This operation was aborted", "AbortError");
  }

  class AbortSignal extends EventTarget {
    constructor(token) {
      super();
      if (token !== signalToken) throw new TypeError("Illegal constructor");
      states.set(this, { aborted: false, reason: undefined });
      this.onabort = null;
    }
    static abort(reason) {
      const controller = new AbortController();
      controller.abort(reason);
      return controller.signal;
    }
    static timeout(milliseconds) {
      const delay = Number(milliseconds);
      if (!Number.isFinite(delay) || delay < 0 || delay > 0xffffffff) {
        throw new RangeError("AbortSignal timeout must be an unsigned integer");
      }
      const controller = new AbortController();
      setTimeout(
        () =>
          controller.abort(
            new DOMException("The operation timed out", "TimeoutError"),
          ),
        Math.floor(delay),
      );
      return controller.signal;
    }
    get aborted() {
      return states.get(this).aborted;
    }
    get reason() {
      return states.get(this).reason;
    }
    throwIfAborted() {
      const state = states.get(this);
      if (state.aborted) throw state.reason;
    }
    _abort(reason) {
      const state = states.get(this);
      if (state.aborted) return;
      state.aborted = true;
      state.reason = reason === undefined ? defaultReason() : reason;
      const event = globalThis.__tysel_event("abort");
      this.dispatchEvent(event);
    }
  }

  class AbortController {
    constructor() {
      Object.defineProperty(this, "signal", {
        value: new AbortSignal(signalToken),
        enumerable: true,
      });
    }
    abort(reason) {
      this.signal._abort(reason);
    }
  }

  globalThis.AbortSignal = AbortSignal;
  globalThis.AbortController = AbortController;
})();
