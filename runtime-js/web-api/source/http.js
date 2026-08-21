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

  class Request {
    constructor(input, init) {
      init = init || {};
      if (typeof input === "string") {
        this.url = input;
        this.method = String(init.method || "GET").toUpperCase();
        this.headers = new Headers(init.headers);
        this.body = init.body == null ? null : init.body;
        this._stream = init.bodyStream === true;
        this._text = null;
        this.signal = init.signal || null;
      } else {
        if (input.bodyUsed && init.body == null) {
          throw new TypeError("cannot construct from a consumed Request");
        }
        this.url = input.url;
        this.method = String(init.method || input.method || "GET").toUpperCase();
        this.headers = new Headers(init.headers || input.headers);
        this.body = init.body == null ? input.body : init.body;
        this._stream = init.bodyStream === true || (init.body == null && input._stream === true);
        this._text = input._text || null;
        this.signal = init.signal || input.signal || null;
      }
      this._bodyUsed = false;
    }
    get bodyUsed() {
      return this._bodyUsed;
    }
    async text() {
      if (this._bodyUsed) throw new TypeError("body has already been consumed");
      this._bodyUsed = true;
      if (this._text != null) return this._text;
      if (this._stream) {
        const chunks = [];
        for (;;) {
          const chunk = await tysel.readBody();
          if (chunk == null) break;
          chunks.push(chunk);
        }
        this._text = chunks.join("");
        this._stream = false;
        return this._text;
      }
      this._text = this.body == null ? "" : String(this.body);
      return this._text;
    }
    async json() {
      const text = await this.text();
      return text ? JSON.parse(text) : null;
    }
    async arrayBuffer() {
      return new TextEncoder().encode(await this.text()).buffer;
    }
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
      this.body = body == null ? null : body;
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
    async text() {
      if (this._bodyUsed) throw new TypeError("body has already been consumed");
      this._bodyUsed = true;
      if (this._stream) {
        const chunks = [];
        try {
          for (;;) {
            const operation = tysel._httpRead(this._bodyId);
            const chunk = await globalThis.__tysel_awaitOperation(
              operation,
              this._signal,
            );
            if (chunk == null) break;
            chunks.push(chunk);
          }
          this.body = chunks.join("");
          return this.body;
        } finally {
          this._stream = false;
          if (this._abortCleanup) this._abortCleanup();
        }
      }
      return this.body == null ? "" : String(this.body);
    }
    async json() {
      const text = await this.text();
      return text ? JSON.parse(text) : null;
    }
    async arrayBuffer() {
      return new TextEncoder().encode(await this.text()).buffer;
    }
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
