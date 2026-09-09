(() => {
  function headerName(name) {
    const key = String(name).toLowerCase();
    if (!/^[!#$%&'*+.^_`|~0-9a-z-]+$/.test(key)) throw new TypeError("invalid header name");
    return key;
  }
  function headerValue(value) {
    const text = String(value).replace(/^[ \t]+|[ \t]+$/g, "");
    if (/[\r\n\0]/.test(text)) throw new TypeError("invalid header value");
    return text;
  }
  function headerIterator(headers, kind) {
    let version = -1, entries, index = 0;
    return {
      [Symbol.iterator]() { return this; },
      next() {
        // Web IDL indexes the current sorted list, including separate cookies.
        // Rebuild only after mutation. Reaching the end does not freeze the list.
        if (version !== headers._version) {
          entries = [];
          for (const key of Object.keys(headers._map).sort()) {
            if (key === "set-cookie") {
              for (const cookie of headers._cookies) entries.push([key, cookie]);
            } else entries.push([key, headers._map[key]]);
          }
          version = headers._version;
        }
        if (index >= entries.length) return {value: undefined, done: true};
        const pair = entries[index++];
        return {value: kind === "keys" ? pair[0] : kind === "values" ? pair[1] : pair, done: false};
      },
    };
  }

  class Headers {
    constructor(init) {
      this._map = Object.create(null);
      this._cookies = [];
      this._version = 0;
      if (init == null) return;
      if (typeof init[Symbol.iterator] === "function") {
        for (const item of init) {
          const pair = Array.from(item);
          if (pair.length !== 2) throw new TypeError("header pair must have two items");
          this.append(pair[0], pair[1]);
        }
      } else {
        for (const key of Object.keys(init)) this.append(key, init[key]);
      }
    }
    get(name) { return this._map[headerName(name)] ?? null; }
    getSetCookie() { return this._cookies.slice(); }
    set(name, value) {
      if (this._immutable) throw new TypeError("headers are immutable");
      const key = headerName(name), text = headerValue(value);
      this._map[key] = text;
      this._version++;
      if (key === "set-cookie") this._cookies = [text];
    }
    append(name, value) {
      if (this._immutable) throw new TypeError("headers are immutable");
      const key = headerName(name), text = headerValue(value);
      const prev = this._map[key];
      this._map[key] = prev == null ? text : prev + ", " + text;
      this._version++;
      if (key === "set-cookie") this._cookies.push(text);
    }
    has(name) { return Object.hasOwn(this._map, headerName(name)); }
    delete(name) {
      if (this._immutable) throw new TypeError("headers are immutable");
      const key = headerName(name);
      delete this._map[key];
      this._version++;
      if (key === "set-cookie") this._cookies = [];
    }
    entries() { return headerIterator(this, "entries"); }
    keys() { return headerIterator(this, "keys"); }
    values() { return headerIterator(this, "values"); }
    forEach(callback, thisArg) { for (const [key, value] of this) callback.call(thisArg, value, key, this); }
    [Symbol.iterator]() { return this.entries(); }
  }

  function bodyBytes(body) {
    if (globalThis.__tysel_isReadableStream(body)) throw new TypeError("streaming uploads are not supported");
    if (body == null) return new Uint8Array(0);
    if (Array.isArray(body)) {
      const chunks = body.map(bodyBytes);
      return joinBytes(chunks, chunks.reduce((size, chunk) => size + chunk.byteLength, 0));
    }
    if (body instanceof ArrayBuffer) return new Uint8Array(body);
    if (ArrayBuffer.isView(body)) {
      return new Uint8Array(body.buffer, body.byteOffset, body.byteLength);
    }
    return new TextEncoder().encode(String(body));
  }

  function copyBody(body) {
    return body instanceof ArrayBuffer || ArrayBuffer.isView(body)
      ? bodyBytes(body).slice()
      : body;
  }

  function copyResponseBody(body) {
    if (!Array.isArray(body)) return copyBody(body);
    // Snapshot both the chunk list and each view's selected bytes. Native
    // emission can then handle every public ArrayBufferView as Uint8Array.
    return Array.from(body, (chunk) => {
      if (typeof chunk !== "string" && !(chunk instanceof ArrayBuffer) && !ArrayBuffer.isView(chunk)) {
        throw new TypeError("response chunk must be a string or BufferSource");
      }
      return copyBody(chunk);
    });
  }

  function joinBytes(chunks, length) {
    if (chunks.length === 1) return chunks[0];
    const bytes = new Uint8Array(length);
    let offset = 0;
    for (const chunk of chunks) {
      bytes.set(chunk, offset);
      offset += chunk.byteLength;
    }
    return bytes;
  }

  function used(owner) {
    // Adapter to the pinned web-streams-polyfill's disturbed flag. Covered by
    // reader/pipe/async-iteration lifecycle tests when updating the vendor.
    return owner._bodyUsed || Boolean(owner._bodyStream && owner._bodyStream._disturbed);
  }
  function getBody(owner) {
    if (owner._bodyStream) return owner._bodyStream;
    if (!owner._stream && owner._body == null) return null;
    if (owner._bodyUsed) {
      // The buffered fast path consumed the body without initializing Streams.
      // Materialize the same closed, disturbed, unlocked state as consumeBytes.
      // A public read marks disturbance synchronously without vendor field writes.
      const stream = new ReadableStream({ start(controller) { controller.close(); } });
      const reader = stream.getReader();
      reader.read();
      reader.releaseLock();
      owner._bodyStream = stream;
      return stream;
    }
    const host = owner._stream;
    const generation = owner._generation;
    let operation = null;
    let index = 0;
    const chunks = Array.isArray(owner._body) ? owner._body : [owner._body];
    owner._bodyStream = new ReadableStream({
      async pull(controller) {
        if (used(owner) && owner._bodyUsed) throw new TypeError("body has already been consumed");
        if (host) {
          if (generation !== globalThis.__tysel_request_generation) throw new TypeError("body belongs to a completed request");
          operation = owner instanceof Request ? tysel._readBodyOp() : tysel._httpRead(owner._bodyId);
          try {
            const chunk = await globalThis.__tysel_awaitOperation(operation, owner._signal || owner.signal);
            if (chunk == null) {
              controller.close();
              if (owner._abortCleanup) owner._abortCleanup();
            } else controller.enqueue(chunk);
          } finally { operation = null; }
        } else {
          if (index < chunks.length) controller.enqueue(bodyBytes(chunks[index++]));
          if (index === chunks.length) controller.close();
        }
      },
      cancel() {
        if (host && generation === globalThis.__tysel_request_generation) {
          if (operation) tysel._cancelOp(operation.id);
          if (owner instanceof Request) tysel._cancelRequestBody();
          else tysel._httpCancelBody(owner._bodyId);
        }
        if (owner._abortCleanup) owner._abortCleanup();
      },
    }, { highWaterMark: 0 });
    return owner._bodyStream;
  }
  async function consumeBytes(owner) {
    if (used(owner)) throw new TypeError("body has already been consumed");
    if (!owner._stream && !owner._bodyStream) {
      if (owner._body != null) owner._bodyUsed = true;
      return bodyBytes(owner._body);
    }
    const reader = getBody(owner).getReader();
    const chunks = [];
    let length = 0;
    try {
      for (;;) {
        const {value: chunk, done} = await reader.read();
        if (done) break;
        if (!(chunk instanceof Uint8Array)) throw new TypeError("HTTP stream chunks must be Uint8Array");
        if (chunk.byteLength) { chunks.push(chunk); length += chunk.byteLength; }
      }
      return joinBytes(chunks, length);
    } catch (error) {
      // Cleanup must not delay the original failure if user cancellation hangs.
      try { reader.cancel(error).catch(() => {}); } catch (_) {}
      throw error;
    } finally { owner._bodyUsed = true; reader.releaseLock(); }
  }
  async function consumeText(owner) {
    if (!owner._stream && !owner._bodyStream && typeof owner._body === "string") {
      if (used(owner)) throw new TypeError("body has already been consumed");
      owner._bodyUsed = true;
      return owner._body.charCodeAt(0) === 0xfeff ? owner._body.slice(1) : owner._body;
    }
    return new TextDecoder().decode(await consumeBytes(owner));
  }
  async function consumeArrayBuffer(owner) {
    const ownsBytes = owner._stream || owner._bodyStream || (Array.isArray(owner._body) && owner._body.length !== 1);
    const bytes = await consumeBytes(owner);
    // A user-supplied stream can retain its chunks: return independent bytes.
    return ownsBytes && !owner._customStream && bytes.byteOffset === 0 && bytes.byteLength === bytes.buffer.byteLength
      ? bytes.buffer : bytes.buffer.slice(bytes.byteOffset, bytes.byteOffset + bytes.byteLength);
  }

  globalThis.__tysel_bodyBytes = bodyBytes;
  globalThis.__tysel_consumeBytes = consumeBytes;

  class Request {
    constructor(input, init) {
      init = init || {};
      if (globalThis.__tysel_isReadableStream(init.body)) throw new TypeError("streaming uploads are not supported");
      if (typeof input === "string") {
        this.url = input;
        this.method = String(init.method || "GET").toUpperCase();
        this.headers = new Headers(init.headers);
        this._body = init.body == null ? null : copyBody(init.body);
        this._stream = init.bodyStream === true;
        this.signal = init.signal || null;
      } else {
        if ((input.bodyUsed || (input._bodyStream && input._bodyStream.locked)) && init.body == null) {
          throw new TypeError("cannot construct from a consumed Request");
        }
        this.url = input.url;
        this.method = String(init.method || input.method || "GET").toUpperCase();
        this.headers = new Headers(init.headers || input.headers);
        this._body = copyBody(init.body == null ? input._body : init.body);
        this._stream = init.bodyStream === true || (init.body == null && input._stream === true);
        this.signal = init.signal || input.signal || null;
      }
      this._generation = typeof input !== "string" && init.body == null && input._stream
        ? input._generation : globalThis.__tysel_request_generation;
      this._bodyUsed = false;
      this._bodyStream = null;
      this._customStream = false;
    }
    get bodyUsed() {
      return used(this);
    }
    get body() { return getBody(this); }
    async text() { return consumeText(this); }
    async json() { return JSON.parse(await this.text()); }
    async arrayBuffer() { return consumeArrayBuffer(this); }
    async bytes() { return new Uint8Array(await consumeArrayBuffer(this)); }
    clone() {
      if (this._stream || this._customStream || used(this) || (this._bodyStream && this._bodyStream.locked)) {
        throw new TypeError("cannot clone a streaming or consumed request");
      }
      return new Request(this.url, {
        method: this.method,
        headers: this.headers,
        body: this._body,
        signal: this.signal,
      });
    }
  }

  class Response {
    constructor(body, init) {
      init = init || {};
      const stream = globalThis.__tysel_isReadableStream(body);
      if (stream && (body.locked || body._disturbed)) throw new TypeError("body stream is locked or consumed");
      this._body = stream ? null : body == null ? null : copyResponseBody(body);
      let status = 200;
      const initialStatus = init.status;
      if (initialStatus !== undefined) {
        const number = +initialStatus;
        status = Number.isFinite(number) ? ((Math.trunc(number) % 65536) + 65536) % 65536 : 0;
        if ((status < 200 || status > 599) && !(status === 101 && globalThis.__tysel_ws_accepted)) throw new RangeError("invalid response status");
        if (body != null && (status === 101 || status === 204 || status === 205 || status === 304)) throw new TypeError("status cannot have a body");
      }
      this.status = status;
      this.headers = new Headers(init.headers);
      this._stream = false;
      this._signal = null;
      this._generation = globalThis.__tysel_request_generation;
      this._bodyUsed = false;
      this._bodyStream = null;
      this._customStream = stream;
      if (this._customStream) this._bodyStream = body;
    }
    get type() { return this.status === 0 ? "error" : "default"; }
    get ok() {
      return this.status >= 200 && this.status < 300;
    }
    static error() {
      const response = new Response();
      response.status = 0;
      response.headers._immutable = true;
      return response;
    }
    static redirect(url, status = 302) {
      const location = new URL(String(url)).href;
      const number = +status;
      status = Number.isFinite(number) ? ((Math.trunc(number) % 65536) + 65536) % 65536 : 0;
      if (![301, 302, 303, 307, 308].includes(status)) throw new RangeError("invalid redirect status");
      const response = new Response(null, {status, headers: {location}});
      response.headers._immutable = true;
      return response;
    }
    static json(data, init) {
      init = init || {};
      const body = JSON.stringify(data);
      if (body === undefined) throw new TypeError("data is not JSON serializable");
      // The constructor snapshots headers once. Building another Headers here
      // would iterate, sort and revalidate the same list a second time.
      const response = new Response(body, init);
      if (!response.headers.get("content-type")) {
        response.headers.set("content-type", "application/json");
      }
      return response;
    }
    get bodyUsed() {
      return used(this);
    }
    get body() { return getBody(this); }
    async text() { return consumeText(this); }
    async json() { return JSON.parse(await this.text()); }
    async arrayBuffer() { return consumeArrayBuffer(this); }
    async bytes() { return new Uint8Array(await consumeArrayBuffer(this)); }
    clone() {
      if (this._stream || this._customStream || used(this) || (this._bodyStream && this._bodyStream.locked)) {
        throw new TypeError("cannot clone a streaming or consumed response");
      }
      if (this.status === 0) return Response.error();
      const response = new Response(this._body, { status: this.status, headers: this.headers });
      response.headers._immutable = this.headers._immutable;
      return response;
    }
  }

  globalThis.__tysel_responseBody = response => {
    if (response.status === 0) throw new TypeError("cannot send a network-error Response");
    if (used(response)) throw new TypeError("response body has already been consumed");
    if (response._bodyStream && response._bodyStream.locked) throw new TypeError("response body is locked");
    return response._stream || response._bodyStream ? response.body : response._body;
  };
  globalThis.__tysel_headerPairs = headers => Array.from(headers);
  globalThis.__tysel_pumpResponse = async function(body, write, closed) {
    const reader = body.getReader();
    let stopped = false;
    const pump = async () => {
      for (;;) {
        const {value, done} = await reader.read();
        if (done || stopped) break;
        if (!(value instanceof Uint8Array)) throw new TypeError("HTTP stream chunks must be Uint8Array");
        await write(value);
      }
    };
    try {
      // One race per response, not per chunk: pending reactions stay bounded.
      await Promise.race([pump(), closed.promise.then(() => {
        throw new TypeError("response consumer closed");
      })]);
    } catch (error) {
      stopped = true;
      // Invoke cleanup, but an application's pending cancel promise must not
      // keep the worker assigned after the consumer has disconnected.
      try { reader.cancel(error).catch(() => {}); } catch (_) {}
      throw error;
    } finally {
      stopped = true;
      tysel._cancelOp(closed.id);
      reader.releaseLock();
    }
  };
  globalThis.Headers = Headers;
  globalThis.Request = Request;
  globalThis.Response = Response;
})();
