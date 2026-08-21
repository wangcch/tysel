(() => {
  class CryptoKey {
    constructor(token, type, extractable, algorithm, usages) {
      if (token !== cryptoKeyToken) throw new TypeError("Illegal constructor");
      Object.defineProperties(this, {
        type: { value: type, enumerable: true },
        extractable: { value: extractable, enumerable: true },
        algorithm: { value: Object.freeze(algorithm), enumerable: true },
        usages: { value: Object.freeze(usages), enumerable: true },
      });
      Object.freeze(this);
    }
  }
  globalThis.CryptoKey = CryptoKey;
  const integerTypedArrays = new Set([
    "[object Int8Array]",
    "[object Uint8Array]",
    "[object Uint8ClampedArray]",
    "[object Int16Array]",
    "[object Uint16Array]",
    "[object Int32Array]",
    "[object Uint32Array]",
    "[object BigInt64Array]",
    "[object BigUint64Array]",
  ]);
  globalThis.crypto = {
    getRandomValues(typedArray) {
      if (!integerTypedArrays.has(Object.prototype.toString.call(typedArray))) {
        throw new DOMException("expected an integer TypedArray", "TypeMismatchError");
      }
      const view = new Uint8Array(
        typedArray.buffer,
        typedArray.byteOffset,
        typedArray.byteLength,
      );
      if (view.byteLength > 65536) {
        throw new DOMException("random value request exceeds 65536 bytes", "QuotaExceededError");
      }
      view.set(tysel._randomBytes(view.byteLength));
      return typedArray;
    },
    subtle: {
      async digest(algorithm, data) {
        const name = normalizeCryptoHash(algorithm);
        return tysel._digest(name, toCryptoBytes(data)).buffer;
      },
      async importKey(format, keyData, algorithm, extractable, keyUsages) {
        if (format !== "raw") throw new DOMException("only raw CryptoKeys are supported", "NotSupportedError");
        const algo = typeof algorithm === "string" ? { name: algorithm } : algorithm || {};
        if (String(algo.name).toUpperCase() !== "HMAC") throw new DOMException("only HMAC keys are supported", "NotSupportedError");
        const hash = normalizeCryptoHash(algo.hash || "SHA-256");
        const usages = Array.from(keyUsages || [], String);
        if (usages.length === 0) {
          throw new DOMException("secret keys require at least one usage", "SyntaxError");
        }
        if (usages.some((usage) => usage !== "sign" && usage !== "verify")) {
          throw new DOMException("HMAC keys only support sign and verify usages", "SyntaxError");
        }
        if (new Set(usages).size !== usages.length) {
          throw new DOMException("HMAC key usages must not contain duplicates", "SyntaxError");
        }
        const bytes = new Uint8Array(toCryptoBytes(keyData));
        const sourceLength = bytes.byteLength * 8;
        if (sourceLength === 0) throw new DOMException("HMAC key data cannot be empty", "DataError");
        let length = sourceLength;
        if (algo.length !== undefined) {
          length = Number(algo.length);
          if (!Number.isInteger(length) || length < 0 || length > 0xffffffff) {
            throw new TypeError("HMAC key length must be an unsigned integer");
          }
          if (length > sourceLength || length <= sourceLength - 8) {
            throw new DOMException("HMAC key length is inconsistent with key data", "DataError");
          }
          if (length % 8 !== 0) {
            bytes[bytes.length - 1] &= (0xff << (8 - (length % 8))) & 0xff;
          }
        }
        // Ask the native implementation to validate the hash before returning a key.
        tysel._digest(hash, new Uint8Array(0));
        const key = new CryptoKey(
          cryptoKeyToken,
          "secret",
          Boolean(extractable),
          { name: "HMAC", hash: Object.freeze({ name: hash }), length },
          usages,
        );
        cryptoKeys.set(key, { hash, bytes });
        return key;
      },
      async sign(algorithm, key, data) {
        const rec = cryptoKeys.get(key);
        if (!rec) throw new TypeError("unknown CryptoKey");
        const name = typeof algorithm === "string" ? algorithm : String(algorithm && algorithm.name || "");
        if (name.toUpperCase() !== "HMAC") throw new DOMException("only HMAC signing is supported", "NotSupportedError");
        if (!key.usages.includes("sign")) throw new DOMException("key does not allow signing", "InvalidAccessError");
        return tysel._hmac(rec.hash, rec.bytes, toCryptoBytes(data)).buffer;
      },
      async verify(algorithm, key, signature, data) {
        const rec = cryptoKeys.get(key);
        if (!rec) throw new TypeError("unknown CryptoKey");
        const name = typeof algorithm === "string" ? algorithm : String(algorithm && algorithm.name || "");
        if (name.toUpperCase() !== "HMAC") throw new DOMException("only HMAC verification is supported", "NotSupportedError");
        if (!key.usages.includes("verify")) throw new DOMException("key does not allow verification", "InvalidAccessError");
        return tysel._hmacVerify(rec.hash, rec.bytes, toCryptoBytes(signature), toCryptoBytes(data));
      },
    },
  };
  const cryptoKeyToken = Object.freeze({});
  const cryptoKeys = new WeakMap();
  function normalizeCryptoHash(algorithm) {
    const raw = typeof algorithm === "string" ? algorithm : String(algorithm && algorithm.name || "");
    const compact = raw.trim().toUpperCase().replace("-", "");
    if (compact === "SHA256") return "SHA-256";
    if (compact === "SHA384") return "SHA-384";
    if (compact === "SHA512") return "SHA-512";
    throw new DOMException(`unsupported digest algorithm ${raw}`, "NotSupportedError");
  }
  function toCryptoBytes(data) {
    if (data instanceof ArrayBuffer) return new Uint8Array(data);
    if (ArrayBuffer.isView(data)) {
      return new Uint8Array(data.buffer, data.byteOffset, data.byteLength);
    }
    throw new TypeError("expected BufferSource");
  }
})();
