(() => {
  function abortReason(signal) {
    return signal.reason === undefined
      ? new DOMException("This operation was aborted", "AbortError")
      : signal.reason;
  }

  function awaitOperation(operation, signal) {
    if (!signal) return operation.promise;
    if (signal.aborted) {
      tysel._cancelOp(operation.id);
      return Promise.reject(abortReason(signal));
    }
    return new Promise((resolve, reject) => {
      let settled = false;
      let aborted = false;
      const onAbort = () => {
        if (settled) return;
        aborted = true;
        tysel._cancelOp(operation.id);
      };
      signal.addEventListener("abort", onAbort, { once: true });
      operation.promise.then(
        (value) => {
          if (settled) return;
          settled = true;
          signal.removeEventListener("abort", onAbort);
          if (aborted || signal.aborted) reject(abortReason(signal));
          else resolve(value);
        },
        (error) => {
          if (settled) return;
          settled = true;
          signal.removeEventListener("abort", onAbort);
          reject(signal.aborted ? abortReason(signal) : error);
        },
      );
    });
  }

  globalThis.__tysel_awaitOperation = awaitOperation;

  globalThis.fetch = async function(input, init) {
    init = init || {};
    const signal = init.signal || (input && input.signal) || null;
    if (signal && signal.aborted) throw abortReason(signal);
    const url = typeof input === "string" ? input : input.url;
    const method = String(init.method || (input && input.method) || "GET").toUpperCase();
    const headers = new Headers(init.headers || (input && input.headers));
    const pairs = [];
    headers.forEach((value, key) => pairs.push([key, value]));
    let body = "";
    if (init.body != null) body = String(init.body);
    else if (input instanceof Request && (input._stream || input.body != null)) body = await input.text();
    else if (input && typeof input !== "string" && input.body != null) body = String(input.body);
    const operation = tysel._httpStart(String(url), method, JSON.stringify(pairs), body);
    const started = await awaitOperation(operation, signal);
    let headerPairs = [];
    try { headerPairs = JSON.parse(started.headers || "[]"); } catch (_) {}
    const response = new Response(null, { status: started.status, headers: headerPairs });
    response._stream = true;
    response._signal = signal;
    response._bodyId = started.bodyId;
    if (signal) {
      const cancelBody = () => tysel._httpCancelBody(response._bodyId);
      signal.addEventListener("abort", cancelBody, { once: true });
      response._abortCleanup = () => signal.removeEventListener("abort", cancelBody);
    }
    return response;
  };
  globalThis.tysel.httpGet = function(url) {
    return fetch(url);
  };
})();
