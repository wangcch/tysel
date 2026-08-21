import app from "app.js";

const names = [];
if (typeof app === "function") {
  names.push("default");
} else if (
  app &&
  app.durable &&
  typeof app.durable === "object" &&
  !Array.isArray(app.durable)
) {
  for (const name of Object.keys(app.durable).sort()) {
    if (typeof app.durable[name] === "function") names.push(name);
  }
}
globalThis.__tysel_durable_exports_json = JSON.stringify(names);
