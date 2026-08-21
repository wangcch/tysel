  const durable = {
    step(name, fn) {
      return boundary("step", name, fn);
    },
    effect(name, fn) {
      return boundary("effect", name, fn);
    },
    now() {
      enter();
      try {
        const replay = lookup("now", "now");
        if (replay.found) return new Date(replay.payload);
        const value = Date.now();
        tysel._durableRecord("now", "now", encode(value), value);
        return new Date(value);
      } finally {
        active = false;
      }
    },
    random() {
      enter();
      try {
        const replay = lookup("random", "random");
        if (replay.found) return replay.payload;
        const value = Math.random();
        tysel._durableRecord("random", "random", encode(value), Date.now());
        return value;
      } finally {
        active = false;
      }
    },
    async sleep(duration) {
      const millis = durationMs(duration);
      const key = "sleep:" + millis;
      enter();
      try {
        const replay = lookup("sleep", key);
        if (replay.found) {
          tysel._durableCompleteSleep();
          return;
        }
        const now = Date.now();
        const wakeAt = now + millis;
        if (!Number.isSafeInteger(wakeAt)) throw new TypeError("durable wakeup is too large");
        tysel._durableRecordSleep(key, encode({ durationMs: millis }), now, wakeAt);
        await tysel.sleep(millis);
        tysel._durableCompleteSleep();
      } finally {
        active = false;
      }
    },
    async waitForSignal(name) {
      const key = String(name);
      if (!key) throw new TypeError("durable signal name cannot be empty");
      enter();
      try {
        const replay = lookup("signal", key);
        if (replay.found) return replay.payload;
        const signal = JSON.parse(tysel._durablePollSignal(key));
        if (signal.found) return signal.payload;
        await new Promise(() => {});
      } finally {
        active = false;
      }
    },
    async retry(policyValue, fn) {
      if (typeof fn !== "function") throw new TypeError("durable retry requires a function");
      const policy = retryPolicy(policyValue);
      const retryId = retryIndex++;
      const scope = [
        "retry",
        retryId,
        policy.maxAttempts,
        policy.delayMs,
        policy.factor,
        policy.maxDelayMs === null ? "none" : policy.maxDelayMs,
      ].join(":");
      for (let attempt = 1; attempt <= policy.maxAttempts; attempt++) {
        retryLookupOrRecord(scope + ":start:" + attempt, { attempt });
        const outcomeKey = scope + ":outcome:" + attempt;
        const replayedOutcome = findRetryOutcome(outcomeKey);
        if (replayedOutcome.found) {
          const outcome = replayedOutcome.payload;
          if (outcome && outcome.ok === true) return applyRetryOutcome(outcome);
          if (attempt === policy.maxAttempts) return applyRetryOutcome(outcome);
          const delayMs = retryDelayMs(policy, attempt);
          if (delayMs > 0) await durable.sleep(delayMs);
          continue;
        }
        let failed = false;
        let failure;
        let value;
        try {
          value = await fn(attempt);
        } catch (error) {
          failed = true;
          failure = retryFailure(error);
        }
        const outcome = failed ? { ok: false, failure } : { ok: true, value };
        retryLookupOrRecord(outcomeKey, outcome);
        if (!failed) return applyRetryOutcome(outcome);
        if (attempt === policy.maxAttempts) throwRetryFailure(failure);
        const delayMs = retryDelayMs(policy, attempt);
        if (delayMs > 0) await durable.sleep(delayMs);
      }
      throw new Error("durable retry exhausted unexpectedly");
    },
  };
  globalThis.tysel.durable = durable;
