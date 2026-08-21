(() => {
  if (!Number.isInteger(globalThis.__tysel_request_generation)) {
    globalThis.__tysel_request_generation = 0;
  }
  class WebSocket extends EventTarget {
    static CONNECTING = 0;
    static OPEN = 1;
    static CLOSING = 2;
    static CLOSED = 3;
    constructor(url) {
      super();
      this.url = String(url);
      this._generation = globalThis.__tysel_request_generation;
      this.readyState = WebSocket.CONNECTING;
      this.binaryType = "arraybuffer";
      this.onopen = null;
      this.onmessage = null;
      this.onerror = null;
      this.onclose = null;
      this.opened = tysel._wsConnect(this.url).then(() => {
        if (this._generation !== globalThis.__tysel_request_generation) return this;
        this.readyState = WebSocket.OPEN;
        this._dispatch("open", { type: "open", target: this });
        this._readLoop();
        return this;
      }, (error) => {
        if (this._generation !== globalThis.__tysel_request_generation) return this;
        this.readyState = WebSocket.CLOSED;
        this._dispatch("error", { type: "error", error, target: this });
        this._dispatch("close", { type: "close", code: 1006, reason: String(error), wasClean: false, target: this });
        throw error;
      });
    }
    send(data) {
      if (this.readyState !== WebSocket.OPEN) throw new DOMException("WebSocket is not open", "InvalidStateError");
      return tysel._wsClientSend(data == null ? "" : String(data));
    }
    async close() {
      if (this.readyState === WebSocket.CLOSED || this.readyState === WebSocket.CLOSING) return;
      this.readyState = WebSocket.CLOSING;
      await tysel._wsClientClose();
    }
    _dispatch(type, init) {
      if (this._generation !== globalThis.__tysel_request_generation) return;
      const event = globalThis.__tysel_event(type, init);
      this.dispatchEvent(event);
    }
    async _readLoop() {
      try {
        while (this.readyState === WebSocket.OPEN) {
          const frame = await tysel._wsClientRead();
          if (this._generation !== globalThis.__tysel_request_generation) return;
          if (frame.type === "close") {
            this.readyState = WebSocket.CLOSED;
            this._dispatch("close", {
              type: "close",
              code: frame.code,
              reason: frame.reason,
              wasClean: frame.wasClean,
              target: this,
            });
            return;
          }
          let data = frame.data;
          if (Array.isArray(data)) data = Uint8Array.from(data).buffer;
          this._dispatch("message", { type: "message", data, target: this });
        }
        this.readyState = WebSocket.CLOSED;
        this._dispatch("close", { type: "close", code: 1000, reason: "", wasClean: true, target: this });
      } catch (error) {
        if (this._generation !== globalThis.__tysel_request_generation) return;
        this.readyState = WebSocket.CLOSED;
        this._dispatch("error", { type: "error", error, target: this });
        this._dispatch("close", { type: "close", code: 1006, reason: String(error), wasClean: false, target: this });
      }
    }
  }
  for (const name of ["CONNECTING", "OPEN", "CLOSING", "CLOSED"]) {
    Object.defineProperty(WebSocket.prototype, name, { value: WebSocket[name] });
  }
  globalThis.WebSocket = WebSocket;
})();
