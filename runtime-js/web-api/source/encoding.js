(() => {
  // Read the internal view length even when an instance or subclass shadows it.
  const byteLength = Object.getOwnPropertyDescriptor(Object.getPrototypeOf(Uint8Array.prototype), "byteLength").get;
  class TextEncoder {
    constructor() {
      this.encoding = "utf-8";
    }
    encodeInto(source, destination) {
      if (arguments.length < 2) throw new TypeError("encodeInto requires source and destination");
      source = `${source}`;
      if (!(destination instanceof Uint8Array)) throw new TypeError("expected Uint8Array");
      const capacity = byteLength.call(destination);
      let read = 0, written = 0;
      // Bound each native conversion, including a surrogate lookahead.
      do {
        let end = Math.min(source.length, read + Math.min(16384, capacity - written + 1));
        if (end < source.length && end > read + 1 && source.charCodeAt(end - 1) >= 0xd800 && source.charCodeAt(end - 1) <= 0xdbff) end--;
        const chunk = source.slice(read, end);
        const result = tysel._utf8EncodeInto(chunk, destination, written);
        read += result.read; written += result.written;
        if (!result.read || result.read < chunk.length) break;
      } while (read < source.length && written < capacity);
      return { read, written };
    }
    encode(input) {
      const text = input === undefined ? "" : `${input}`;
      try { return tysel._utf8Encode(text); }
      catch (error) {
        // The native bridge rejects lone surrogate code points. Keep valid
        // strings on the existing fast path; normalize only this fallback.
        const scalar = text.replace(/[\uD800-\uDBFF][\uDC00-\uDFFF]|[\uD800-\uDFFF]/g, unit => unit.length === 2 ? unit : "\ufffd");
        if (scalar === text) throw error;
        return tysel._utf8Encode(scalar);
      }
    }
  }

  class TextDecoder {
    constructor(label, options) {
      const encoding = String(label == null ? "utf-8" : label)
        .trim()
        .toLowerCase()
        .replace(/[_-]/g, "");
      if (encoding !== "utf8") {
        throw new RangeError("TextDecoder only supports utf-8");
      }
      this.encoding = "utf-8";
      this.fatal = Boolean(options && options.fatal);
      this.ignoreBOM = Boolean(options && options.ignoreBOM);
      this._pending = new Uint8Array(0);
      this._bomSeen = false;
    }
    decode(input, options) {
      let view;
      if (input == null) view = new Uint8Array(0);
      else if (input instanceof ArrayBuffer) view = new Uint8Array(input);
      else if (ArrayBuffer.isView(input)) view = new Uint8Array(input.buffer, input.byteOffset, input.byteLength);
      else throw new TypeError("expected BufferSource");
      const stream = Boolean(options && options.stream);
      if (this._pending.length) {
        const joined = new Uint8Array(this._pending.length + view.byteLength);
        joined.set(this._pending); joined.set(view, this._pending.length); view = joined;
      }
      this._pending = new Uint8Array(0);
      if (stream && view.length) {
        let start = view.length - 1;
        while (start > 0 && view[start] >= 128 && view[start] <= 191 && view.length - start < 4) start--;
        const lead = view[start];
        const required = lead >= 194 && lead <= 223 ? 2 : lead >= 224 && lead <= 239 ? 3 : lead >= 240 && lead <= 244 ? 4 : 0;
        const available = view.length - start;
        const second = view[start + 1];
        const validSecond = available < 2 || (second >= 128 && second <= 191
          && !(lead === 224 && second < 160) && !(lead === 237 && second > 159)
          && !(lead === 240 && second < 144) && !(lead === 244 && second > 143));
        if (required > available && validSecond) {
          this._pending = view.slice(start);
          view = view.subarray(0, start);
        }
      }
      try {
        let text = tysel._utf8Decode(view, this.fatal);
        if (text.length && !this._bomSeen) {
          this._bomSeen = true;
          if (!this.ignoreBOM && text.charCodeAt(0) === 0xfeff) text = text.slice(1);
        }
        if (!stream) { this._pending = new Uint8Array(0); this._bomSeen = false; }
        return text;
      } catch (error) {
        this._pending = new Uint8Array(0); this._bomSeen = false;
        throw error;
      }
    }
  }

  class TextDecoderStream {
    constructor(label, options) {
      const decoder = new TextDecoder(label, options);
      const transform = new TransformStream({
        transform(chunk, controller) {
          const text = decoder.decode(chunk, {stream: true});
          if (text) controller.enqueue(text);
        },
        flush(controller) { const text = decoder.decode(); if (text) controller.enqueue(text); },
      });
      for (const key of ["encoding", "fatal", "ignoreBOM"]) Object.defineProperty(this, key, {value: decoder[key], enumerable: true});
      Object.defineProperties(this, {readable: {value: transform.readable}, writable: {value: transform.writable}});
    }
  }
  globalThis.TextDecoderStream = TextDecoderStream;

  globalThis.TextEncoder = TextEncoder;
  globalThis.TextDecoder = TextDecoder;
})();
