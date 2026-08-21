if (!globalThis.tysel.durable) globalThis.tysel.durable = {};
globalThis.tysel.durable.start = function(name, input) {
  return JSON.parse(tysel._durableStart(String(name), JSON.stringify(input === undefined ? null : input)));
};
globalThis.tysel.durable.sendSignal = function(taskId, name, payload) {
  tysel._durableSendSignal(String(taskId), String(name), JSON.stringify(payload === undefined ? null : payload));
};
