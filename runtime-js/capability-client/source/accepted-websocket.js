(() => {
  class AcceptedWebSocket extends EventTarget {
    constructor() {
      super();
      this.readyState = 1;
      this.onmessage = null;
      this.onclose = null;
      this.onerror = null;
    }
    send(data) {
      return tysel._wsSend(data == null ? "" : String(data));
    }
    close() {
      this.readyState = 2;
      return tysel._wsClose();
    }
    _dispatch(type, init) {
      const event = globalThis.__tysel_event(type, init);
      this.dispatchEvent(event);
    }
  }
  globalThis.tysel.acceptWebSocket = function() {
    if (globalThis.__tysel_ws_accepted) {
      throw new Error("websocket already accepted");
    }
    globalThis.__tysel_ws_accepted = true;
    const socket = new AcceptedWebSocket();
    globalThis.__tysel_ws_done = (async () => {
      let close = { code: 1000, reason: "", wasClean: true };
      try {
        for (;;) {
          const chunk = await tysel._wsRead();
          if (chunk == null) break;
          socket._dispatch("message", { data: chunk });
        }
      } catch (error) {
        close = { code: 1006, reason: String(error), wasClean: false };
        socket._dispatch("error", { error });
      } finally {
        socket.readyState = 3;
        socket._dispatch("close", close);
      }
    })();
    return socket;
  };
})();
