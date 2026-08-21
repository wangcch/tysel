(() => {
  const eventStates = new WeakMap();
  const targetStates = new WeakMap();

  function isListener(listener) {
    return typeof listener === "function" ||
      (listener != null && typeof listener.handleEvent === "function");
  }

  function callListener(listener, target, event) {
    try {
      if (typeof listener === "function") listener.call(target, event);
      else listener.handleEvent(event);
    } catch {
      // Listener failures do not propagate through dispatchEvent and cannot
      // terminate native I/O pumps.
    }
  }

  function captureOption(options) {
    return typeof options === "boolean" ? options : Boolean(options && options.capture);
  }

  class Event {
    constructor(type, init) {
      if (arguments.length === 0) throw new TypeError("Event type is required");
      init = init || {};
      eventStates.set(this, {
        target: null,
        currentTarget: null,
        dispatching: false,
        stopped: false,
        immediate: false,
        canceled: false,
      });
      Object.defineProperties(this, {
        type: { value: String(type), enumerable: true },
        bubbles: { value: Boolean(init.bubbles), enumerable: true },
        cancelable: { value: Boolean(init.cancelable), enumerable: true },
        composed: { value: Boolean(init.composed), enumerable: true },
        timeStamp: { value: Date.now(), enumerable: true },
      });
    }
    get target() { return eventStates.get(this).target; }
    get currentTarget() { return eventStates.get(this).currentTarget; }
    get defaultPrevented() { return eventStates.get(this).canceled; }
    stopPropagation() { eventStates.get(this).stopped = true; }
    stopImmediatePropagation() {
      const state = eventStates.get(this);
      state.stopped = true;
      state.immediate = true;
    }
    preventDefault() {
      if (this.cancelable) eventStates.get(this).canceled = true;
    }
  }

  class EventTarget {
    constructor() {
      targetStates.set(this, []);
    }
    addEventListener(type, listener, options) {
      if (!isListener(listener)) return;
      type = String(type);
      const capture = captureOption(options);
      const entries = targetStates.get(this);
      if (entries.some((entry) =>
        entry.type === type && entry.listener === listener && entry.capture === capture
      )) return;
      const entry = {
        type,
        listener,
        capture,
        once: Boolean(options && typeof options === "object" && options.once),
      };
      entries.push(entry);
      const signal = options && typeof options === "object" ? options.signal : null;
      if (signal) {
        if (signal.aborted) this.removeEventListener(type, listener, capture);
        else signal.addEventListener("abort", () => this.removeEventListener(type, listener, capture), { once: true });
      }
    }
    removeEventListener(type, listener, options) {
      const capture = captureOption(options);
      const entries = targetStates.get(this);
      type = String(type);
      targetStates.set(this, entries.filter((entry) =>
        entry.type !== type || entry.listener !== listener || entry.capture !== capture
      ));
    }
    dispatchEvent(event) {
      if (!(event instanceof Event)) throw new TypeError("expected Event");
      const state = eventStates.get(event);
      if (state.dispatching || !event.type) throw new DOMException("event is already being dispatched", "InvalidStateError");
      state.dispatching = true;
      state.target = this;
      state.currentTarget = this;
      state.stopped = false;
      state.immediate = false;
      try {
        const handler = this[`on${event.type}`];
        if (isListener(handler)) callListener(handler, this, event);
        const entries = targetStates.get(this).filter((entry) => entry.type === event.type);
        for (const entry of entries) {
          if (state.immediate) break;
          if (entry.once) this.removeEventListener(entry.type, entry.listener, entry.capture);
          callListener(entry.listener, this, event);
        }
      } finally {
        state.currentTarget = null;
        state.dispatching = false;
      }
      return !state.canceled;
    }
  }

  globalThis.Event = Event;
  globalThis.EventTarget = EventTarget;
  globalThis.__tysel_event = function(type, init) {
    const event = new Event(type, init);
    for (const [key, value] of Object.entries(init || {})) {
      if (!(key in event)) Object.defineProperty(event, key, { value, enumerable: true });
    }
    return event;
  };
})();
