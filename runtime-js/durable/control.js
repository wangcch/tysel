if (!globalThis.tysel.durable) globalThis.tysel.durable = {};
function durableRequestKey(options) {
  if (options === undefined) return undefined;
  if (!options || typeof options.idempotencyKey !== "string" || !options.idempotencyKey) {
    throw new TypeError("durable options require a nonempty idempotencyKey");
  }
  return options.idempotencyKey;
}
globalThis.tysel.durable.start = function(name, input, options) {
  return JSON.parse(tysel._durableStart(String(name), JSON.stringify(input === undefined ? null : input), durableRequestKey(options)));
};
globalThis.tysel.durable.sendSignal = function(taskId, name, payload, options) {
  tysel._durableSendSignal(String(taskId), String(name), JSON.stringify(payload === undefined ? null : payload), durableRequestKey(options));
};
