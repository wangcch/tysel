import { denied } from "./audit.js";
import type { DurableContext } from "@tysel/types";
import { Failure, fields, jsonBody, text, type Config, type Snapshot } from "./protocol.js";
import { accept, attempt, claim, finish, live, reserve, type Definition, type Job, type Outcome } from "./store.js";

export async function config(): Promise<Config> {
  return JSON.parse(await tysel.fs.read("config/service.json")) as Config;
}
export function compatible(job: Job, cfg: Config): boolean {
  const definition = JSON.parse(job.definition_json) as Definition;
  return definition.callerDigest === cfg.callerDigest && definition.pluginDigest === cfg.pluginDigest && definition.adapterId === cfg.adapterId;
}
async function authority(job: Job, step: string, ticketId?: string): Promise<Config> {
  await live(job);
  const cfg = await config();
  if (!compatible(job, cfg)) throw new Failure("VERSION_MISMATCH");
  if (cfg.bootId !== job.boot_id) throw new Failure("OWNERSHIP_LOST");
  if (step === "read" && !cfg.customers.find((c) => c.id === job.customer)?.tickets.some((t) => t.id === ticketId)) {
    throw new Failure("FORBIDDEN_TICKET");
  }
  return cfg;
}
async function call(job: Job, url: string, init: RequestInit, limit: number): Promise<unknown> {
  const definition = JSON.parse(job.definition_json) as Definition;
  const remaining = job.deadline_at - Date.now();
  if (remaining <= 0) throw new Failure("DEADLINE_EXCEEDED");
  const controller = new AbortController();
  const timer = setTimeout(() => controller.abort(), Math.min(remaining, definition.limits.callMs));
  try {
    const response = await fetch(url, { ...init, redirect: "manual", signal: controller.signal });
    if (!response.ok) {
      await response.body?.cancel();
      throw new Failure(response.status === 429 || response.status >= 500 ? "UPSTREAM_RETRYABLE" : "UPSTREAM_REJECTED");
    }
    return await jsonBody(response, limit);
  } catch (error) {
    if (controller.signal.aborted) throw new Failure("CALL_TIMEOUT");
    if (error instanceof Failure) throw error;
    throw new Failure("UPSTREAM_UNAVAILABLE");
  } finally { clearTimeout(timer); }
}
function validatePlugin(job: Job, step: string, reply: unknown): unknown {
  const snapshot = JSON.parse(job.input_json) as Snapshot;
  if (step === "select" && snapshot.tickets.length > 0) {
    if (!fields(reply, ["kind", "operation", "ticketId"]) || reply.kind !== "lookup" ||
        reply.operation !== "ticket.read" || !text(reply.ticketId, 128)) throw new Failure("FORBIDDEN_OPERATION");
    const selected = [...snapshot.tickets].sort((a, b) => b.priority - a.priority)[0]!;
    if (selected.id !== reply.ticketId) throw new Failure("FORBIDDEN_TICKET");
  } else if (!fields(reply, ["kind", "customerId", "summary"]) || reply.kind !== "done" ||
      reply.customerId !== job.customer || !text(reply.summary, 1024)) throw new Failure("PROTOCOL_ERROR");
  return reply;
}
async function physical(job: Job, step: string, ordinal: number, payload: unknown): Promise<Outcome> {
  const existing = await attempt(job, step, ordinal);
  if (existing?.outcome_json) return JSON.parse(existing.outcome_json) as Outcome;
  const ticketId = step === "read" ? (payload as { ticketId: string }).ticketId : undefined;
  try {
    await authority(job, step, ticketId);
    if (existing) {
      // A reservation with unknown outcome is burned, never dispatched a second time.
      const outcome: Outcome = { ok: false, error: "UNKNOWN_OUTCOME", retry: true };
      await accept(job, existing, outcome);
      return outcome;
    }
    const slot = await reserve(job, step, ordinal);
    // Recheck at the real dispatch boundary, after the reservation and any host work.
    const secret = step === "read" ? await tysel.secrets.ref("TRIAGE_FIXTURE_TOKEN") : null;
    const cfg = await authority(job, step, ticketId);
    let value: unknown;
    try {
      if (step === "read") {
        const detail = await call(job, cfg.adapterOrigin + "/tickets/" + encodeURIComponent(ticketId!), {
          headers: { authorization: `Bearer ${secret}`, "x-customer-id": job.customer },
        }, 16384);
        if (!detail || typeof detail !== "object" || Array.isArray(detail)) throw new Failure("INVALID_ADAPTER_RESULT");
        const record = detail as Record<string, unknown>;
        if (record.ticketId !== ticketId || !text(record.subject, 500)) throw new Failure("INVALID_ADAPTER_RESULT");
        value = { ticketId, subject: record.subject };
      } else {
        const envelope = { protocolVersion: 1, jobId: job.id, stepId: step === "select" ? "1" : "2", attemptId: slot.attempt_id, payload };
        const body = JSON.stringify(envelope);
        if (new TextEncoder().encode(body).byteLength > 16384) throw new Failure("MESSAGE_TOO_LARGE");
        const reply = await call(job, cfg.pluginOrigin + "/triage/v1", {
          method: "POST", headers: { "content-type": "application/json" }, body,
        }, 8192);
        if (!fields(reply, ["protocolVersion", "jobId", "stepId", "attemptId", "payload"]) ||
            reply.protocolVersion !== 1 || reply.jobId !== job.id || reply.stepId !== envelope.stepId || reply.attemptId !== slot.attempt_id) {
          throw new Failure("PROTOCOL_ERROR");
        }
        value = validatePlugin(job, step, reply.payload);
      }
    } catch (error) {
      if (!(error instanceof Failure)) throw error;
      const outcome: Outcome = { ok: false, error: error.code,
        retry: ["CALL_TIMEOUT", "UPSTREAM_RETRYABLE", "UPSTREAM_UNAVAILABLE"].includes(error.code) };
      await accept(job, slot, outcome);
      return outcome;
    }
    const outcome: Outcome = { ok: true, value };
    await accept(job, slot, outcome);
    return outcome;
  } catch (error) {
    if (!(error instanceof Failure)) throw error; // Never turn a storage exception into success.
    await denied(job, step, ordinal, error.code);
    if (error.code === "OWNERSHIP_LOST") throw error;
    return { ok: false, error: error.code, retry: false };
  }
}
async function stored<T>(ctx: DurableContext, name: string, operation: () => Promise<T>): Promise<T> {
  // Application SQLite failures are ordinary JS errors, not runtime-history failures.
  // Bounded storage retries reconcile the journal before any new physical dispatch.
  try {
    return await ctx.retry({ maxAttempts: 3, delay: 250, factor: 1 },
      (retry) => ctx.effect(`${name}:storage:${retry}`, async () => {
        try { return await operation(); }
        catch (error) {
          if (error instanceof Failure) throw error;
          const failure = new Error("APPLICATION_STORAGE_UNAVAILABLE");
          failure.name = "ApplicationStorageUnavailable";
          throw failure;
        }
      }));
  } catch (error) {
    if (error instanceof Error && error.name === "ApplicationStorageUnavailable") throw new Failure("STORAGE_UNAVAILABLE");
    // Runtime history errors are outside the callback and retain runtime fencing/recovery.
    throw error;
  }
}
async function operation(ctx: DurableContext, job: Job, step: string, payload: unknown): Promise<unknown> {
  const definition = JSON.parse(job.definition_json) as Definition;
  for (let ordinal = 1; ordinal <= definition.attempts; ordinal++) {
    if (ordinal > 1) await ctx.sleep(definition.retryMs);
    const outcome = await stored(ctx, `${step}:${ordinal}`, () => physical(job, step, ordinal, payload));
    if (outcome.ok) return outcome.value;
    if (!outcome.retry) throw new Failure(outcome.error);
  }
  throw new Failure("ATTEMPTS_EXHAUSTED");
}
export async function triage(ctx: DurableContext, input: { jobId: string }): Promise<Record<string, unknown>> {
  const cfg = await config();
  const job = await claim(input.jobId, cfg.bootId);
  const snapshot = JSON.parse(job.input_json) as Snapshot;
  let result: unknown = null, failure: string | null = null;
  try {
    let reply = await operation(ctx, job, "select", snapshot) as Record<string, unknown>;
    if (snapshot.tickets.length > 0) {
      const detail = await operation(ctx, job, "read", { ticketId: reply.ticketId });
      await stored(ctx, "project-detail", async () => {
        await tysel.sqlite.exec(`UPDATE triage_jobs SET detail_json = ?
          WHERE id = ? AND owner = ? AND state = 'running' AND deadline_at > ? AND detail_json IS NULL`,
        [JSON.stringify(detail), job.id, job.owner, Date.now()]);
        return null;
      });
      reply = await operation(ctx, job, "summarize", { ...snapshot, detail }) as Record<string, unknown>;
    }
    result = { customerId: job.customer, summary: reply.summary };
  } catch (error) {
    if (!(error instanceof Failure) || error.code === "OWNERSHIP_LOST") throw error;
    failure = error.code;
  }
  return await stored(ctx, "terminal", () => finish(job, result, failure));
}
