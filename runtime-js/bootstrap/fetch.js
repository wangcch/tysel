import handler from "app.js";

if (handler == null || typeof handler.fetch !== "function") {
  throw new TypeError("module must export default { fetch }");
}
globalThis.__tysel_fetch = function(request) {
  return handler.fetch.call(handler, request, globalThis.tysel);
};
