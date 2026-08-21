import app from "app.js";

const registry = app && app.tasks;
const name = globalThis.__tysel_task_name;
const task = registry && registry[name];
if (task == null || typeof task.handler !== "function") {
  throw new TypeError("unknown module task: " + name);
}
if (task.kind !== "cron" && task.kind !== "queue" && task.kind !== "mcp") {
  throw new TypeError("task " + name + " has an unsupported kind");
}
const input = JSON.parse(globalThis.__tysel_task_input_json);
const context = Object.freeze({
  requestId: globalThis.__tysel_task_request_id,
  deadlineMs: globalThis.__tysel_task_deadline_ms,
});
const value =
  task.kind === "cron"
    ? await task.handler.call(task, context)
    : await task.handler.call(task, input, context);
const encoded = JSON.stringify(value === undefined ? null : value);
if (encoded === undefined) {
  throw new TypeError("task result must be JSON serializable");
}
globalThis.__tysel_task_value_json = encoded;
