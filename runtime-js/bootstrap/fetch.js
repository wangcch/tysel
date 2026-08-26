import handler from "app.js";

if (handler == null || typeof handler.fetch !== "function") {
  throw new TypeError("module must export default { fetch }");
}

const registry = handler.tasks;
if (registry == null) {
  globalThis.__tysel_task_manifest_json = "[]";
} else {
  if (typeof registry !== "object" || Array.isArray(registry)) {
    throw new TypeError("module tasks must be an object");
  }
  const definitions = [];
  for (const name of Object.keys(registry).sort()) {
    const task = registry[name];
    if (
      task == null ||
      typeof task !== "object" ||
      typeof task.handler !== "function"
    ) {
      throw new TypeError("task " + name + " must define a handler");
    }
    if (task.kind === "cron") {
      definitions.push({
        name,
        kind: "cron",
        expression: String(task.expression || ""),
      });
    } else if (task.kind === "queue") {
      definitions.push({ name, kind: "queue", queue: String(task.name || "") });
    } else if (task.kind === "mcp") {
      definitions.push({
        name,
        kind: "mcp",
        description: String(task.description || ""),
        input: task.input == null ? {} : task.input,
      });
    } else {
      throw new TypeError("task " + name + " has an unsupported kind");
    }
  }
  globalThis.__tysel_task_manifest_json = JSON.stringify(definitions);
}

const durableNames = [];
if (typeof handler === "function") {
  durableNames.push("default");
} else if (
  handler.durable &&
  typeof handler.durable === "object" &&
  !Array.isArray(handler.durable)
) {
  for (const name of Object.keys(handler.durable).sort()) {
    if (typeof handler.durable[name] === "function") durableNames.push(name);
  }
}
globalThis.__tysel_durable_exports_json = JSON.stringify(durableNames);

globalThis.__tysel_fetch = function(request) {
  return handler.fetch.call(handler, request, globalThis.tysel);
};
