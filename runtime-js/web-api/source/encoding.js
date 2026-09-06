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
    }
    decode(input) {
      if (input == null) return "";
      let view;
      if (input instanceof ArrayBuffer) view = new Uint8Array(input);
      else if (ArrayBuffer.isView(input)) {
        view = new Uint8Array(input.buffer, input.byteOffset, input.byteLength);
      } else {
        throw new TypeError("expected BufferSource");
      }
      let text = tysel._utf8Decode(view, this.fatal);
      if (!this.ignoreBOM && text.charCodeAt(0) === 0xfeff) text = text.slice(1);
      return text;
    }
  }

  globalThis.TextEncoder = TextEncoder;
  globalThis.TextDecoder = TextDecoder;
})();
