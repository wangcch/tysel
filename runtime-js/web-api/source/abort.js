(() => {
  const signalToken = Object.freeze({});
  const states = new WeakMap();

  // Held values contain only weak source references, never the dependent.
  const collected = new FinalizationRegistry(links => {
    for (const [source, dependent] of links) {
      const signal = source.deref();
      if (signal) states.get(signal).followers.delete(dependent);
    }
  });

  function defaultReason() {
    return new DOMException("This operation was aborted", "AbortError");
  }

  class AbortSignal extends EventTarget {
    constructor(token) {
      super();
      if (token !== signalToken) throw new TypeError("Illegal constructor");
      states.set(this, { aborted: false, reason: undefined, followers: new Set() });
      this.onabort = null;
    }
    static abort(reason) {
      const controller = new AbortController();
      controller.abort(reason);
      return controller.signal;
    }
    static any(signals) {
      if (signals == null || typeof signals[Symbol.iterator] !== "function") {
        throw new TypeError("expected an iterable of AbortSignal");
      }
      const list = Array.from(signals);
      for (const signal of list) if (!states.has(signal)) throw new TypeError("expected AbortSignal");
      const controller = new AbortController();
      const aborted = list.find(signal => signal.aborted);
      if (aborted) { controller.abort(aborted.reason); return controller.signal; }
      const dependent = new WeakRef(controller.signal);
      const links = [];
      for (const signal of new Set(list)) {
        states.get(signal).followers.add(dependent);
        links.push([new WeakRef(signal), dependent]);
      }
      const state = states.get(controller.signal);
      state.links = links;
      // A live dependent keeps its propagation path alive, including temporary
      // intermediate composites. The reverse edges remain weak.
      state.sources = list;
      // This QuickJS version retains unregister tokens: never use the target.
      if (links.length) collected.register(controller.signal, links, links);
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
      const pending = [this];
      const value = reason === undefined ? defaultReason() : reason;
      // Mark the complete dependency graph before running any user callback.
      state.aborted = true;
      state.reason = value;
      for (let i = 0; i < pending.length; i++) {
        const current = states.get(pending[i]);
        for (const reference of current.followers) {
          const signal = reference.deref();
          if (!signal) continue;
          const next = states.get(signal);
          if (next.aborted) continue;
          next.aborted = true;
          next.reason = value;
          pending.push(signal);
        }
        current.followers.clear();
      }
      for (const signal of pending) {
        const current = states.get(signal);
        for (const [source, dependent] of current.links || []) {
          const parent = source.deref();
          if (parent) states.get(parent).followers.delete(dependent);
        }
        if (current.links) collected.unregister(current.links);
        current.links = undefined;
        current.sources = undefined;
      }
      for (const signal of pending) signal.dispatchEvent(globalThis.__tysel_event("abort"));
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
