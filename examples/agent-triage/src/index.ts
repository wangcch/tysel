import type { TyselApp } from "@tysel/types";
import { admission, Failure, fields, hash, jsonBody, text, ticketIds } from "./protocol.js";
import { admit, byKey, find, initialize, publicJob } from "./store.js";
import { config, triage } from "./workflow.js";

export default {
  durable: { triage },
  async fetch(request) {
    try {
      const cfg = await config();
      await initialize(cfg);
      const path = new URL(request.url).pathname;
      if (path === "/health" && request.method === "GET") return Response.json({ ready: true });
      if (path.startsWith("/internal/")) {
        if (request.headers.get("authorization") !== `Bearer ${cfg.dispatchToken}`) throw new Failure("UNAUTHORIZED", 401);
        if (path === "/internal/pending" && request.method === "GET") {
          return Response.json(await tysel.sqlite.query("SELECT id FROM triage_jobs WHERE execution_version = 2 AND delivery = 'pending' ORDER BY delivery_checked_at,created_at LIMIT 8"));
        }
        if (path === "/internal/dispatch" && request.method === "POST") {
          const input = await jsonBody(request, 256);
          if (!fields(input, ["jobId"]) || !text(input.jobId, 64)) throw new Failure("INVALID_INPUT");
          const job = await find(input.jobId);
          if (!job || job.execution_version !== 2 || job.delivery !== "pending") return Response.json({ dispatched: false });
          // Rotate unresolved acknowledgements so they cannot starve newer jobs.
          await tysel.sqlite.exec("UPDATE triage_jobs SET delivery_checked_at = ? WHERE id = ?", [Date.now(), job.id]);
          if (JSON.parse(job.definition_json).callerDigest !== cfg.callerDigest) throw new Failure("VERSION_MISMATCH", 503);
          // Retry the same export, full pinned bundle and immutable input after an uncertain acknowledgement.
          const started = tysel.durable.start("triage", { jobId: job.id }, { idempotencyKey: "triage.v2:" + job.id });
          await tysel.sqlite.exec(`UPDATE triage_jobs SET task_id = ?, delivery = ? WHERE id = ? AND delivery = 'pending'`,
            [started.taskId, started.status === "completed" ? "completed" : "pending", job.id]);
          return Response.json({ dispatched: true, status: started.status });
        }
        throw new Failure("NOT_FOUND", 404);
      }
      const customer = cfg.customers.find((customer) => request.headers.get("authorization") === `Bearer ${customer.token}`);
      if (!customer) throw new Failure("UNAUTHORIZED", 401);
      if (path === "/jobs" && request.method === "POST") {
        const key = request.headers.get("idempotency-key");
        if (!key || !/^[\x21-\x7e]{1,128}$/.test(key)) throw new Failure("INVALID_IDEMPOTENCY_KEY");
        const ids = ticketIds(await jsonBody(request, 16384));
        const canonical = JSON.stringify({ protocolVersion: 1, ticketIds: ids });
        const keyHash = await hash(JSON.stringify([customer.id, "triage.v1", key]));
        // Compare the original client input before projecting current data/policy.
        const existing = await byKey(customer.id, keyHash);
        if (existing && existing.request_json !== canonical) throw new Failure("IDEMPOTENCY_CONFLICT", 409);
        const job = existing ?? await admit(cfg, admission(ids, customer), keyHash, canonical);
        return Response.json(publicJob(job), { status: 202 });
      }
      const route = path.match(/^\/jobs\/([a-f0-9-]{36})$/);
      if (route && request.method === "GET") {
        const job = await find(route[1]!);
        if (!job || job.customer !== customer.id) throw new Failure("NOT_FOUND", 404);
        return Response.json(publicJob(job));
      }
      throw new Failure("NOT_FOUND", 404);
    } catch (error) {
      return Response.json({ error: error instanceof Failure ? error.code : "STORAGE_UNAVAILABLE" },
        { status: error instanceof Failure ? error.status : 503 });
    }
  },
} satisfies TyselApp;
