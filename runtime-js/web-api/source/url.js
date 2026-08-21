(() => {
  function decodeURIComponentSafe(value) {
    try {
      return decodeURIComponent(String(value).replace(/\+/g, " "));
    } catch {
      return String(value);
    }
  }

  function encodeFormComponent(value) {
    return encodeURIComponent(String(value))
      .replace(/[!'()~]/g, (character) =>
        "%" + character.charCodeAt(0).toString(16).toUpperCase(),
      )
      .replace(/%20/g, "+");
  }

  function splitReference(value) {
    const hashIndex = value.indexOf("#");
    const hash = hashIndex === -1 ? "" : value.slice(hashIndex);
    const withoutHash = hashIndex === -1 ? value : value.slice(0, hashIndex);
    const searchIndex = withoutHash.indexOf("?");
    return {
      pathname: searchIndex === -1 ? withoutHash : withoutHash.slice(0, searchIndex),
      search: searchIndex === -1 ? "" : withoutHash.slice(searchIndex),
      hash,
    };
  }

  function normalizePath(pathname) {
    const absolute = pathname.charAt(0) === "/";
    const trailing = /\/(?:\.{0,2})?$/.test(pathname);
    const segments = [];
    for (const segment of pathname.split("/")) {
      if (!segment || segment === ".") continue;
      if (segment === "..") segments.pop();
      else segments.push(segment);
    }
    let result = (absolute ? "/" : "") + segments.join("/");
    if (absolute && !result) result = "/";
    if (trailing && result !== "/") result += "/";
    return result;
  }

  function parseAbsolute(value) {
    const match = /^([a-zA-Z][a-zA-Z0-9+.-]*:)\/\/([^/?#]*)([^?#]*)(\?[^#]*)?(#.*)?$/.exec(value);
    if (!match) throw new TypeError("Invalid URL");
    const protocol = match[1].toLowerCase();
    const authority = match[2];
    const host = authority.slice(authority.lastIndexOf("@") + 1);
    let hostname = host;
    let port = "";
    if (host.charAt(0) === "[") {
      const bracket = host.indexOf("]");
      if (bracket === -1) throw new TypeError("Invalid URL");
      hostname = host.slice(0, bracket + 1);
      if (host.charAt(bracket + 1) === ":") port = host.slice(bracket + 2);
    } else {
      const colon = host.lastIndexOf(":");
      if (colon !== -1) {
        hostname = host.slice(0, colon);
        port = host.slice(colon + 1);
      }
    }
    if (!hostname) throw new TypeError("Invalid URL");
    const pathname = normalizePath(match[3] || "/");
    const origin = protocol + "//" + authority;
    return {
      protocol,
      origin,
      host,
      hostname,
      port,
      pathname,
      search: match[4] || "",
      hash: match[5] || "",
      href: origin + pathname + (match[4] || "") + (match[5] || ""),
    };
  }

  function resolveUrl(value, base) {
    if (/^[a-zA-Z][a-zA-Z0-9+.-]*:/.test(value)) return parseAbsolute(value);
    if (base == null) throw new TypeError("Invalid URL");
    const parsedBase = parseAbsolute(String(base));
    if (value.slice(0, 2) === "//") return parseAbsolute(parsedBase.protocol + value);

    const reference = splitReference(value);
    let pathname;
    let search = reference.search;
    if (!reference.pathname) {
      pathname = parsedBase.pathname;
      if (!search && value.charAt(0) !== "?") search = parsedBase.search;
    } else if (reference.pathname.charAt(0) === "/") {
      pathname = normalizePath(reference.pathname);
    } else {
      const slash = parsedBase.pathname.lastIndexOf("/");
      pathname = normalizePath(parsedBase.pathname.slice(0, slash + 1) + reference.pathname);
    }
    return parseAbsolute(parsedBase.origin + pathname + search + reference.hash);
  }

  class URLSearchParams {
    constructor(init, update) {
      this._pairs = [];
      this._update = typeof update === "function" ? update : null;
      this._replace(init);
    }
    _replace(init) {
      this._pairs = [];
      if (init == null) return;
      if (typeof init === "string") {
        const text = init.charAt(0) === "?" ? init.slice(1) : init;
        if (!text) return;
        for (const part of text.split("&")) {
          if (!part) continue;
          const eq = part.indexOf("=");
          if (eq === -1) this._pairs.push([decodeURIComponentSafe(part), ""]);
          else {
            this._pairs.push([
              decodeURIComponentSafe(part.slice(0, eq)),
              decodeURIComponentSafe(part.slice(eq + 1)),
            ]);
          }
        }
      } else if (typeof init[Symbol.iterator] === "function") {
        for (const pair of init) {
          if (pair == null || typeof pair[Symbol.iterator] !== "function") {
            throw new TypeError("URLSearchParams sequence entries must be iterable");
          }
          const values = Array.from(pair);
          if (values.length !== 2) {
            throw new TypeError("URLSearchParams sequence entries must contain two items");
          }
          this.append(values[0], values[1]);
        }
      } else {
        for (const key of Object.keys(init)) {
          this._pairs.push([String(key), String(init[key])]);
        }
      }
    }
    _changed() {
      if (this._update) this._update(this.toString());
    }
    get size() {
      return this._pairs.length;
    }
    append(name, value) {
      this._pairs.push([String(name), String(value)]);
      this._changed();
    }
    set(name, value) {
      name = String(name);
      value = String(value);
      const first = this._pairs.findIndex((pair) => pair[0] === name);
      if (first === -1) this._pairs.push([name, value]);
      else {
        this._pairs[first][1] = value;
        this._pairs = this._pairs.filter((pair, index) => pair[0] !== name || index === first);
      }
      this._changed();
    }
    get(name) {
      name = String(name);
      for (const pair of this._pairs) {
        if (pair[0] === name) return pair[1];
      }
      return null;
    }
    getAll(name) {
      name = String(name);
      return this._pairs.filter((pair) => pair[0] === name).map((pair) => pair[1]);
    }
    has(name) {
      name = String(name);
      return this._pairs.some((pair) => pair[0] === name);
    }
    delete(name) {
      name = String(name);
      this._pairs = this._pairs.filter((pair) => pair[0] !== name);
      this._changed();
    }
    sort() {
      this._pairs.sort((left, right) =>
        left[0] < right[0] ? -1 : left[0] > right[0] ? 1 : 0,
      );
      this._changed();
    }
    forEach(callback, thisArg) {
      for (const [name, value] of this._pairs) {
        callback.call(thisArg, value, name, this);
      }
    }
    entries() {
      return this._pairs.map((pair) => pair.slice())[Symbol.iterator]();
    }
    keys() {
      return this._pairs.map((pair) => pair[0])[Symbol.iterator]();
    }
    values() {
      return this._pairs.map((pair) => pair[1])[Symbol.iterator]();
    }
    [Symbol.iterator]() {
      return this.entries();
    }
    toString() {
      return this._pairs
        .map((pair) => encodeFormComponent(pair[0]) + "=" + encodeFormComponent(pair[1]))
        .join("&");
    }
  }

  class URL {
    constructor(url, base) {
      const parsed = resolveUrl(String(url), base);
      this._searchParams = new URLSearchParams(parsed.search, (serialized) => {
        this._search = serialized ? "?" + serialized : "";
        this._commit();
      });
      this._apply(parsed, false);
    }
    _apply(parsed, updateParams = true) {
      this._protocol = parsed.protocol;
      this._host = parsed.host;
      this._hostname = parsed.hostname;
      this._port = parsed.port;
      this._pathname = parsed.pathname;
      this._search = parsed.search;
      this._hash = parsed.hash;
      if (updateParams) this._searchParams._replace(this._search);
      this._commit();
    }
    _commit() {
      this._origin = this._protocol + "//" + this._host;
      this._href = this._origin + this._pathname + this._search + this._hash;
    }
    get href() { return this._href; }
    set href(value) { this._apply(parseAbsolute(String(value))); }
    get protocol() { return this._protocol; }
    set protocol(value) {
      let protocol = String(value);
      if (!protocol.endsWith(":")) protocol += ":";
      if (!/^[a-zA-Z][a-zA-Z0-9+.-]*:$/.test(protocol)) throw new TypeError("Invalid URL protocol");
      this._protocol = protocol.toLowerCase();
      this._commit();
    }
    get origin() { return this._origin; }
    get host() { return this._host; }
    set host(value) {
      const parsed = parseAbsolute(this._protocol + "//" + String(value) + this._pathname);
      this._host = parsed.host;
      this._hostname = parsed.hostname;
      this._port = parsed.port;
      this._commit();
    }
    get hostname() { return this._hostname; }
    set hostname(value) {
      const hostname = String(value);
      const parsed = parseAbsolute(
        this._protocol + "//" + hostname + (this._port ? ":" + this._port : "") + this._pathname,
      );
      this._host = parsed.host;
      this._hostname = parsed.hostname;
      this._commit();
    }
    get port() { return this._port; }
    set port(value) {
      const port = String(value);
      if (port && (!/^\d+$/.test(port) || Number(port) > 65535)) throw new TypeError("Invalid URL port");
      this._port = port;
      this._host = this._hostname + (port ? ":" + port : "");
      this._commit();
    }
    get pathname() { return this._pathname; }
    set pathname(value) {
      let pathname = String(value);
      if (pathname.charAt(0) !== "/") pathname = "/" + pathname;
      this._pathname = normalizePath(pathname);
      this._commit();
    }
    get search() { return this._search; }
    set search(value) {
      const search = String(value);
      this._search = !search ? "" : search.charAt(0) === "?" ? search : "?" + search;
      this._searchParams._replace(this._search);
      this._commit();
    }
    get searchParams() { return this._searchParams; }
    get hash() { return this._hash; }
    set hash(value) {
      const hash = String(value);
      this._hash = !hash ? "" : hash.charAt(0) === "#" ? hash : "#" + hash;
      this._commit();
    }
    toString() {
      return this.href;
    }
    toJSON() {
      return this.href;
    }
  }

  globalThis.URL = URL;
  globalThis.URLSearchParams = URLSearchParams;
})();
