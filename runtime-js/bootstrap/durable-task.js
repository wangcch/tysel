import input from "tysel-task-input.js";
import task from "app.js";

const exportName = String(globalThis.__tysel_durable_export || "");
delete globalThis.__tysel_durable_export;

function resolve(exported) {
  if (typeof exported === "function") {
    if (exportName && exportName !== "default") {
      throw new TypeError(
        "durable task module exports a default function, not " + exportName,
      );
    }
    return exported;
  }
  const table = exported && exported.durable;
  if (!table || typeof table !== "object" || Array.isArray(table)) {
    throw new TypeError(
      "durable task module must export a default function or durable map",
    );
  }
  const name = exportName || Object.keys(table).sort()[0];
  const run = name ? table[name] : undefined;
  if (typeof run !== "function") {
    throw new TypeError("durable export is missing");
  }
  return run;
}

const value = await resolve(task)(globalThis.tysel.durable, input);
const encoded = JSON.stringify(value);
if (encoded === undefined) {
  throw new TypeError("durable task result must be JSON serializable");
}
globalThis.__tysel_task_value_json = encoded;
