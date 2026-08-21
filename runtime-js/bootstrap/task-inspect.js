import app from "app.js";

const registry = app && app.tasks;
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
