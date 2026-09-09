(() => {
  globalThis.queueMicrotask = function(callback) {
    if (typeof callback !== "function") throw new TypeError("callback must be a function");
    const generation = globalThis.__tysel_request_generation;
    tysel._queueMicrotask(() => {
      if (generation === globalThis.__tysel_request_generation) callback();
    });
  };
  const alphabet = "ABCDEFGHIJKLMNOPQRSTUVWXYZabcdefghijklmnopqrstuvwxyz0123456789+/";
  globalThis.btoa = function(input) {
    const text = String(input);
    const output = [];
    for (let i = 0; i < text.length; i += 3) {
      const a = text.charCodeAt(i), b = text.charCodeAt(i + 1), c = text.charCodeAt(i + 2);
      if (a > 255 || b > 255 || c > 255) throw new DOMException("Invalid character", "InvalidCharacterError");
      output.push(alphabet[a >> 2], alphabet[((a & 3) << 4) | ((b || 0) >> 4)],
        i + 1 < text.length ? alphabet[((b & 15) << 2) | ((c || 0) >> 6)] : "=",
        i + 2 < text.length ? alphabet[c & 63] : "=");
    }
    return output.join("");
  };
  globalThis.atob = function(input) {
    let text = String(input).replace(/[\t\n\f\r ]/g, "");
    if (text.length % 4 === 0) text = text.replace(/==?$/, "");
    if (text.length % 4 === 1 || /[^A-Za-z0-9+/]/.test(text)) {
      throw new DOMException("Invalid base64", "InvalidCharacterError");
    }
    let bits = 0, count = 0;
    const output = [];
    for (const char of text) {
      bits = (bits << 6) | alphabet.indexOf(char);
      count += 6;
      if (count >= 8) { count -= 8; output.push(String.fromCharCode((bits >> count) & 255)); }
    }
    return output.join("");
  };
})();
