import handler from "app.js";

if (handler == null || typeof handler.fetch !== "function") {
  throw new TypeError("module must export default { fetch }");
}
globalThis.__tysel_fetch = handler.fetch.bind(handler);
