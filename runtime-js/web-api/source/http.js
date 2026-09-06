(() => {
  class Headers {
    constructor(init) {
      this._map = {};
      if (!init) return;
      // Arrays have forEach, so the sequence form must be checked first.
      if (Array.isArray(init)) {
        for (const pair of init) this.append(pair[0], pair[1]);
      } else if (typeof init.forEach === "function") {
        init.forEach((value, key) => this.append(key, value));
      } else {
        for (const key of Object.keys(init)) this.append(key, init[key]);
      }
    }
    get(name) {
      const value = this._map[String(name).toLowerCase()];
      return value === undefined ? null : value;
    }
    set(name, value) {
      this._map[String(name).toLowerCase()] = String(value);
    }
    append(name, value) {
      const key = String(name).toLowerCase();
      const prev = this._map[key];
      this._map[key] = prev == null ? String(value) : prev + ", " + String(value);
    }
    has(name) {
      return Object.prototype.hasOwnProperty.call(this._map, String(name).toLowerCase());
    }
    delete(name) {
      delete this._map[String(name).toLowerCase()];
    }
    _names() {
      return Object.keys(this._map).sort();
    }
    forEach(callback, thisArg) {
      for (const key of this._names()) {
        callback.call(thisArg, this._map[key], key, this);
      }
    }
    entries() {
      return this._names().map((key) => [key, this._map[key]])[Symbol.iterator]();
    }
    keys() {
      return this._names()[Symbol.iterator]();
    }
    values() {
      return this._names().map((key) => this._map[key])[Symbol.iterator]();
    }
    [Symbol.iterator]() {
      return this.entries();
    }
  }

  function bodyBytes(body) {
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

  async function consumeBytes(owner) {
    if (owner._bodyUsed) throw new TypeError("body has already been consumed");
    owner._bodyUsed = true;
    if (!owner._stream) return bodyBytes(owner.body);
    const chunks = [];
    let length = 0;
    try {
      for (;;) {
        const chunk = owner instanceof Request
          ? await tysel._readBody()
          : await globalThis.__tysel_awaitOperation(
              tysel._httpRead(owner._bodyId), owner._signal,
            );
        if (chunk == null) break;
        if (chunk.byteLength === 0) continue;
        chunks.push(chunk);
        length += chunk.byteLength;
      }
      return joinBytes(chunks, length);
    } finally {
      owner._stream = false;
      if (owner._abortCleanup) owner._abortCleanup();
    }
  }

  async function consumeText(owner) {
    // Buffered strings need no encode/decode round trip.
    if (!owner._stream && typeof owner.body === "string") {
      if (owner._bodyUsed) throw new TypeError("body has already been consumed");
      owner._bodyUsed = true;
      return owner.body.charCodeAt(0) === 0xfeff ? owner.body.slice(1) : owner.body;
    }
    return new TextDecoder().decode(await consumeBytes(owner));
  }

  async function consumeArrayBuffer(owner) {
    const ownsBytes = owner._stream || (Array.isArray(owner.body) && owner.body.length !== 1);
    const bytes = await consumeBytes(owner);
    // Stream chunks and freshly joined arrays belong to this consumption;
    // a single buffered chunk must not expose its mutable backing storage.
    return ownsBytes && bytes.byteOffset === 0 && bytes.byteLength === bytes.buffer.byteLength
      ? bytes.buffer
      : bytes.buffer.slice(bytes.byteOffset, bytes.byteOffset + bytes.byteLength);
  }

  globalThis.__tysel_bodyBytes = bodyBytes;
  globalThis.__tysel_consumeBytes = consumeBytes;

  class Request {
    constructor(input, init) {
      init = init || {};
      if (typeof input === "string") {
        this.url = input;
        this.method = String(init.method || "GET").toUpperCase();
        this.headers = new Headers(init.headers);
        this.body = init.body == null ? null : copyBody(init.body);
        this._stream = init.bodyStream === true;
        this.signal = init.signal || null;
      } else {
        if (input.bodyUsed && init.body == null) {
          throw new TypeError("cannot construct from a consumed Request");
        }
        this.url = input.url;
        this.method = String(init.method || input.method || "GET").toUpperCase();
        this.headers = new Headers(init.headers || input.headers);
        this.body = copyBody(init.body == null ? input.body : init.body);
        this._stream = init.bodyStream === true || (init.body == null && input._stream === true);
        this.signal = init.signal || input.signal || null;
      }
      this._bodyUsed = false;
    }
    get bodyUsed() {
      return this._bodyUsed;
    }
    async text() { return consumeText(this); }
    async json() { return JSON.parse(await this.text()); }
    async arrayBuffer() { return consumeArrayBuffer(this); }
    clone() {
      if (this._stream || this._bodyUsed) {
        throw new TypeError("cannot clone a streaming or consumed request");
      }
      return new Request(this.url, {
        method: this.method,
        headers: this.headers,
        body: this.body,
        signal: this.signal,
      });
    }
  }

  class Response {
    constructor(body, init) {
      init = init || {};
      this.body = body == null ? null : copyResponseBody(body);
      this.status = init.status || 200;
      this.headers = new Headers(init.headers);
      this._stream = false;
      this._signal = null;
      this._bodyUsed = false;
    }
    get ok() {
      return this.status >= 200 && this.status < 300;
    }
    static json(data, init) {
      init = init || {};
      const headers = new Headers(init.headers);
      if (!headers.get("content-type")) {
        headers.set("content-type", "application/json");
      }
      const body = JSON.stringify(data);
      if (body === undefined) throw new TypeError("data is not JSON serializable");
      return new Response(body, { status: init.status || 200, headers });
    }
    get bodyUsed() {
      return this._bodyUsed;
    }
    async text() { return consumeText(this); }
    async json() { return JSON.parse(await this.text()); }
    async arrayBuffer() { return consumeArrayBuffer(this); }
    clone() {
      if (this._stream || this._bodyUsed) {
        throw new TypeError("cannot clone a streaming or consumed response");
      }
      return new Response(this.body, { status: this.status, headers: this.headers });
    }
  }

  globalThis.Headers = Headers;
  globalThis.Request = Request;
  globalThis.Response = Response;
})();
