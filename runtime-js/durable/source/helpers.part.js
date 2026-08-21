  let active = false;
  let retryIndex = 0;

  function lookup(kind, key) {
    return JSON.parse(tysel._durableLookup(kind, key));
  }

  function encode(value) {
    const encoded = JSON.stringify(value);
    if (encoded === undefined) {
      throw new TypeError("durable values must be JSON serializable");
    }
    return encoded;
  }

  function enter() {
    if (active) {
      throw new Error("durable boundaries must be awaited sequentially");
    }
    active = true;
  }

  function durationMs(value) {
    if (typeof value === "number") {
      if (!Number.isFinite(value) || value < 0) throw new TypeError("invalid durable duration");
      return Math.floor(value);
    }
    const match = /^\s*(\d+(?:\.\d+)?)\s*(ms|s|m|h|d)\s*$/.exec(String(value));
    if (!match) throw new TypeError("invalid durable duration");
    const scales = { ms: 1, s: 1000, m: 60000, h: 3600000, d: 86400000 };
    const millis = Number(match[1]) * scales[match[2]];
    if (!Number.isSafeInteger(Math.floor(millis))) throw new TypeError("durable duration is too large");
    return Math.floor(millis);
  }

  function retryPolicy(value) {
    if (value === null || typeof value !== "object" || Array.isArray(value)) {
      throw new TypeError("durable retry policy must be an object");
    }
    const maxAttempts = value.maxAttempts === undefined ? 3 : Number(value.maxAttempts);
    if (!Number.isInteger(maxAttempts) || maxAttempts < 1 || maxAttempts > 100) {
      throw new TypeError("durable retry maxAttempts must be an integer from 1 to 100");
    }
    const delayMs = value.delay === undefined ? 0 : durationMs(value.delay);
    const factor = value.factor === undefined ? 2 : Number(value.factor);
    if (!Number.isFinite(factor) || factor < 1 || factor > 100) {
      throw new TypeError("durable retry factor must be from 1 to 100");
    }
    const maxDelayMs = value.maxDelay === undefined ? null : durationMs(value.maxDelay);
    return { maxAttempts, delayMs, factor, maxDelayMs };
  }

  function retryDelayMs(policy, attempt) {
    const scaled = Math.floor(policy.delayMs * Math.pow(policy.factor, attempt - 1));
    const millis = policy.maxDelayMs === null ? scaled : Math.min(scaled, policy.maxDelayMs);
    if (!Number.isSafeInteger(millis)) {
      throw new TypeError("durable retry delay is too large");
    }
    return millis;
  }

  function retryFailure(error) {
    let name = "Error";
    let message = "retry callback failed";
    try {
      if (error && typeof error.name === "string") name = error.name;
      message = error && typeof error.message === "string" ? error.message : String(error);
    } catch (_) {}
    return { name: name.slice(0, 256), message: message.slice(0, 4096) };
  }

  function throwRetryFailure(failure) {
    if (!failure || typeof failure.name !== "string" || typeof failure.message !== "string") {
      throw new Error("invalid durable retry history");
    }
    const error = new Error(failure.message);
    error.name = failure.name;
    throw error;
  }

  function retryLookupOrRecord(key, payload) {
    enter();
    try {
      const replay = lookup("retry", key);
      if (replay.found) return replay.payload;
      if (payload !== undefined) {
        tysel._durableRecord("retry", key, encode(payload), Date.now());
      }
      return undefined;
    } finally {
      active = false;
    }
  }
  function findRetryOutcome(key) {
    enter();
    try {
      return JSON.parse(tysel._durableFindRetryOutcome(key));
    } finally {
      active = false;
    }
  }

  function applyRetryOutcome(outcome) {
    if (!outcome || typeof outcome.ok !== "boolean") {
      throw new Error("invalid durable retry outcome");
    }
    if (outcome.ok) return outcome.value;
    throwRetryFailure(outcome.failure);
  }

  async function boundary(kind, name, fn) {
    if (typeof fn !== "function") throw new TypeError("durable boundary requires a function");
    enter();
    try {
      const replay = lookup(kind, String(name));
      if (replay.found) return replay.payload;
      const value = await fn();
      tysel._durableRecord(kind, String(name), encode(value), Date.now());
      return value;
    } finally {
      active = false;
    }
  }
