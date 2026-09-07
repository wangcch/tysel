(() => {
  class TextEncoder {
    constructor() {
      this.encoding = "utf-8";
    }
    encode(input) {
      return tysel._utf8Encode(input == null ? "" : String(input));
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
