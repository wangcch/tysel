import { initializeAudit } from "./audit.js";
import { Failure, type Config, type Limits, type Snapshot } from "./protocol.js";

export interface Job {
  id: string; customer: string; key_hash: string; input_json: string; request_json: string;
  state: string; error: string | null; result_json: string | null;
  created_at: number; deadline_at: number; boot_id: string; definition_json: string;
  plugin_calls: number; read_attempts: number; detail_json: string | null;
  execution_version: number; owner: string | null; task_id: string | null;
  delivery: string; delivery_until: number;
}
export type Definition = { pluginDigest: string; callerDigest: string; adapterId: string; limits: Limits; attempts: number; retryMs: number };
export type Outcome = { ok: true; value: unknown } | { ok: false; error: string; retry: boolean };
export interface Attempt { job_id: string; step: string; ordinal: number; owner: string; attempt_id: string; outcome_json: string | null }
export const terminal = (job: Job): boolean => ["succeeded", "failed", "expired"].includes(job.state);

// Per-isolate schema readiness only; authority and maintenance remain live.
let schemaReady = false;

async function initializeSchema(): Promise<void> {
  await tysel.sqlite.exec(`CREATE TABLE IF NOT EXISTS triage_jobs (
    id TEXT PRIMARY KEY, customer TEXT NOT NULL, key_hash TEXT NOT NULL,
    input_json TEXT NOT NULL, request_json TEXT NOT NULL, state TEXT NOT NULL, error TEXT, result_json TEXT,
    created_at INTEGER NOT NULL, deadline_at INTEGER NOT NULL, boot_id TEXT NOT NULL,
    definition_json TEXT NOT NULL, plugin_calls INTEGER NOT NULL DEFAULT 0,
    read_attempts INTEGER NOT NULL DEFAULT 0, detail_json TEXT,
    UNIQUE(customer, key_hash)
  )`);
  // Additive migration preserves P1 terminal records and marks old active work interrupted.
  const additions: Record<string, string> = { execution_version: "INTEGER NOT NULL DEFAULT 1",
    owner: "TEXT", task_id: "TEXT", delivery: "TEXT NOT NULL DEFAULT 'legacy'", delivery_until: "INTEGER NOT NULL DEFAULT 0", delivery_checked_at: "INTEGER NOT NULL DEFAULT 0" };
  const columns = await tysel.sqlite.query("PRAGMA table_info(triage_jobs)");
  for (const [name, type] of Object.entries(additions)) {
    if (columns.some((column) => column.name === name)) continue;
    try { await tysel.sqlite.exec(`ALTER TABLE triage_jobs ADD COLUMN ${name} ${type}`); }
    catch (error) {
      // Concurrent initializers may have added this exact column already.
      if (!(await tysel.sqlite.query("PRAGMA table_info(triage_jobs)")).some((column) => column.name === name)) throw error;
    }
  }
  await tysel.sqlite.exec(`CREATE TABLE IF NOT EXISTS triage_attempts (
    job_id TEXT NOT NULL, step TEXT NOT NULL, ordinal INTEGER NOT NULL CHECK(ordinal BETWEEN 1 AND 2),
    owner TEXT NOT NULL, attempt_id TEXT NOT NULL, outcome_json TEXT,
    PRIMARY KEY(job_id,step,ordinal), CHECK(step IN ('select','read','summarize'))
  )`);
  await initializeAudit();
}

export async function initialize(config: Config): Promise<void> {
  if (!schemaReady) {
    // Interrupted requests may abandon pending host promises. Cache only success,
    // so the next request can retry rather than await an abandoned operation.
    await initializeSchema();
    schemaReady = true;
  }
  // Preserve expiry, boot ownership and delivery ordering on every request.
  const now = Date.now();
  await tysel.sqlite.exec(`UPDATE triage_jobs SET state = 'expired', error = 'DEADLINE_EXCEEDED'
    WHERE state IN ('accepted','running','recovering') AND deadline_at <= ?`, [now]);
  await tysel.sqlite.exec(`UPDATE triage_jobs SET
    state = CASE WHEN execution_version = 1 THEN 'failed' ELSE 'recovering' END,
    error = CASE WHEN execution_version = 1 THEN 'INTERRUPTED' ELSE NULL END, owner = NULL, boot_id = ?
    WHERE state IN ('accepted','running','recovering') AND boot_id != ?`, [config.bootId, config.bootId]);
  await tysel.sqlite.exec(`UPDATE triage_jobs SET delivery = 'unresolved'
    WHERE delivery = 'pending' AND delivery_until <= ?`, [now]);
}

export async function find(id: string): Promise<Job | undefined> {
  return (await tysel.sqlite.query("SELECT * FROM triage_jobs WHERE id = ?", [id]))[0] as unknown as Job | undefined;
}
export async function byKey(customer: string, key: string): Promise<Job | undefined> {
  return (await tysel.sqlite.query("SELECT * FROM triage_jobs WHERE customer = ? AND key_hash = ?", [customer, key]))[0] as unknown as Job | undefined;
}
export async function admit(config: Config, input: Snapshot, key: string, request: string): Promise<Job> {
  const id = crypto.randomUUID(), now = Date.now();
  const definition: Definition = { pluginDigest: config.pluginDigest, callerDigest: config.callerDigest,
    adapterId: config.adapterId, limits: config.limits, attempts: 2, retryMs: 250 };
  // Business admission is also the outbox intent. No cross-database transaction is claimed.
  await tysel.sqlite.exec(`INSERT INTO triage_jobs
    (id,customer,key_hash,input_json,request_json,state,created_at,deadline_at,boot_id,definition_json,
     execution_version,delivery,delivery_until)
    SELECT ?,?,?,?,?,'accepted',?,?,?, ?,2,'pending',?
    WHERE (SELECT count(*) FROM triage_jobs) < ?
      AND (SELECT count(*) FROM triage_jobs WHERE state IN ('accepted','running','recovering')) < ?
      AND (SELECT count(*) FROM triage_jobs WHERE customer = ? AND state IN ('accepted','running','recovering')) < ?
    ON CONFLICT(customer,key_hash) DO NOTHING`,
  [id, input.customerId, key, JSON.stringify(input), request, now, now + config.limits.jobMs,
    config.bootId, JSON.stringify(definition), now + config.limits.jobMs + 120000,
    config.limits.retained, config.limits.active, input.customerId, config.limits.perCustomer]);
  const saved = await byKey(input.customerId, key);
  if (saved) {
    if (saved.request_json !== request) throw new Failure("IDEMPOTENCY_CONFLICT", 409);
    return saved;
  }
  const count = await tysel.sqlite.query("SELECT count(*) AS total FROM triage_jobs");
  throw new Failure(Number(count[0]?.total) >= config.limits.retained ? "RETENTION_FULL" : "CAPACITY_EXCEEDED",
    Number(count[0]?.total) >= config.limits.retained ? 503 : 429);
}
export function publicJob(job: Job): Record<string, unknown> {
  return { jobId: job.id, customerId: job.customer, state: job.state,
    createdAt: job.created_at, deadlineAt: job.deadline_at,
    result: job.result_json === null ? null : JSON.parse(job.result_json), error: job.error };
}
export async function claim(id: string, bootId: string): Promise<Job> {
  const owner = crypto.randomUUID();
  // Fresh on every runtime execution, deliberately NOT a replayed ctx.step value.
  await tysel.sqlite.exec(`UPDATE triage_jobs SET owner = ?, boot_id = ?, state = 'running'
    WHERE id = ? AND execution_version = 2 AND state IN ('accepted','running','recovering') AND deadline_at > ?`,
  [owner, bootId, id, Date.now()]);
  const job = (await find(id))!;
  return { ...job, owner: terminal(job) ? job.owner : owner };
}
export async function live(job: Job): Promise<Job> {
  const current = (await find(job.id))!;
  if (current.owner !== job.owner) throw new Failure("OWNERSHIP_LOST");
  if (terminal(current)) throw new Failure(current.error ?? "ALREADY_TERMINAL");
  if (Date.now() >= current.deadline_at) throw new Failure("DEADLINE_EXCEEDED");
  return current;
}
export async function attempt(job: Job, step: string, ordinal: number): Promise<Attempt | undefined> {
  return (await tysel.sqlite.query("SELECT * FROM triage_attempts WHERE job_id = ? AND step = ? AND ordinal = ?",
    [job.id, step, ordinal]))[0] as unknown as Attempt | undefined;
}
export async function reserve(job: Job, step: string, ordinal: number): Promise<Attempt> {
  const attemptId = crypto.randomUUID();
  const changed = await tysel.sqlite.exec(`INSERT INTO triage_attempts(job_id,step,ordinal,owner,attempt_id)
    SELECT ?,?,?,?,? WHERE EXISTS (SELECT 1 FROM triage_jobs
      WHERE id = ? AND owner = ? AND state = 'running' AND deadline_at > ?)
      AND (SELECT count(*) FROM triage_attempts WHERE job_id = ? AND step = ?) = ?
    ON CONFLICT(job_id,step,ordinal) DO NOTHING`,
  [job.id, step, ordinal, job.owner, attemptId, job.id, job.owner, Date.now(), job.id, step, ordinal - 1]);
  if (changed !== 1) throw new Failure("BUDGET_OR_OWNERSHIP_LOST");
  await tysel.sqlite.exec(`UPDATE triage_jobs SET
    plugin_calls = (SELECT count(*) FROM triage_attempts WHERE job_id = ? AND step != 'read'),
    read_attempts = (SELECT count(*) FROM triage_attempts WHERE job_id = ? AND step = 'read') WHERE id = ?`,
  [job.id, job.id, job.id]);
  return (await attempt(job, step, ordinal))!;
}
export async function accept(job: Job, slot: Attempt, outcome: Outcome): Promise<void> {
  const changed = await tysel.sqlite.exec(`UPDATE triage_attempts SET outcome_json = ?
    WHERE job_id = ? AND step = ? AND ordinal = ? AND owner = ? AND outcome_json IS NULL
      AND EXISTS (SELECT 1 FROM triage_jobs WHERE id = ? AND owner = ? AND state = 'running' AND deadline_at > ?)`,
  [JSON.stringify(outcome), job.id, slot.step, slot.ordinal, slot.owner, job.id, job.owner, Date.now()]);
  if (changed !== 1) { await live(job); throw new Failure("OWNERSHIP_LOST"); }
}
export async function finish(job: Job, result: unknown, error: string | null): Promise<Record<string, unknown>> {
  const now = Date.now();
  await tysel.sqlite.exec(`UPDATE triage_jobs SET state = ?, result_json = ?, error = ?
    WHERE id = ? AND owner = ? AND state = 'running' AND deadline_at > ?`,
  [error ? "failed" : "succeeded", error ? null : JSON.stringify(result), error, job.id, job.owner, now]);
  await tysel.sqlite.exec(`UPDATE triage_jobs SET state = 'expired', error = 'DEADLINE_EXCEEDED'
    WHERE id = ? AND state IN ('accepted','running','recovering') AND deadline_at <= ?`, [job.id, now]);
  const saved = (await find(job.id))!;
  if (!terminal(saved)) throw new Failure("OWNERSHIP_LOST");
  return publicJob(saved);
}
