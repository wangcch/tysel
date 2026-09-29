// Local, bounded metadata log. Never include bodies, credentials, keys or raw errors.
import type { Job } from "./store.js";

const clock = "CAST((julianday('now') - 2440587.5) * 86400000 AS INTEGER)";
const operation = "CASE NEW.step WHEN 'read' THEN 'ticket.read' WHEN 'select' THEN 'plugin.select' ELSE 'plugin.summarize' END";

export async function initializeAudit(): Promise<void> {
  await tysel.sqlite.exec(`CREATE TABLE IF NOT EXISTS triage_audit (
    job_id TEXT NOT NULL, event_key TEXT NOT NULL, step TEXT NOT NULL, operation TEXT NOT NULL,
    ordinal INTEGER NOT NULL CHECK(ordinal BETWEEN 0 AND 2), attempt_id TEXT,
    decision TEXT NOT NULL CHECK(decision IN ('admit','reserve','accept','deny','terminal')),
    outcome TEXT NOT NULL, at_ms INTEGER NOT NULL, PRIMARY KEY(job_id,event_key)
  )`);
  // Triggers share the business mutation's transaction: no recorded permit without
  // a reserved slot, or accepted result without its audit entry. Replays add no rows.
  await tysel.sqlite.exec(`CREATE TRIGGER IF NOT EXISTS triage_audit_admission AFTER INSERT ON triage_jobs BEGIN
    INSERT INTO triage_audit VALUES(NEW.id,'admission','admission','job.create',0,NULL,'admit','ACCEPTED',${clock}); END`);
  await tysel.sqlite.exec(`CREATE TRIGGER IF NOT EXISTS triage_audit_reservation AFTER INSERT ON triage_attempts BEGIN
    INSERT INTO triage_audit VALUES(NEW.job_id,'reserve:'||NEW.step||':'||NEW.ordinal,NEW.step,${operation},
      NEW.ordinal,NEW.attempt_id,'reserve','PENDING',${clock}); END`);
  await tysel.sqlite.exec(`CREATE TRIGGER IF NOT EXISTS triage_audit_outcome AFTER UPDATE OF outcome_json ON triage_attempts
    WHEN OLD.outcome_json IS NULL AND NEW.outcome_json IS NOT NULL BEGIN
    INSERT INTO triage_audit VALUES(NEW.job_id,'outcome:'||NEW.step||':'||NEW.ordinal,NEW.step,${operation},
      NEW.ordinal,NEW.attempt_id,CASE WHEN json_extract(NEW.outcome_json,'$.ok')=1 THEN 'accept' ELSE 'deny' END,
      CASE WHEN json_extract(NEW.outcome_json,'$.ok')=1 THEN 'OK' ELSE json_extract(NEW.outcome_json,'$.error') END,${clock}); END`);
  await tysel.sqlite.exec(`CREATE TRIGGER IF NOT EXISTS triage_audit_terminal AFTER UPDATE OF state ON triage_jobs
    WHEN OLD.state IN ('accepted','running','recovering') AND NEW.state IN ('succeeded','failed','expired') BEGIN
    INSERT INTO triage_audit VALUES(NEW.id,'terminal','terminal','job.complete',0,NULL,'terminal',
      COALESCE(NEW.error,'SUCCEEDED'),${clock}); END`);
}

export async function denied(job: Job, step: string, ordinal: number, code: string): Promise<void> {
  // One denial per logical slot, including stale owners. These names are caller-owned.
  const outcome = /^[A-Z_]{1,64}$/.test(code) ? code : "EXECUTION_FAILED";
  await tysel.sqlite.exec(`INSERT INTO triage_audit(job_id,event_key,step,operation,ordinal,attempt_id,decision,outcome,at_ms)
    SELECT ?,?,?,?, ?, (SELECT attempt_id FROM triage_attempts WHERE job_id=? AND step=? AND ordinal=?),'deny',?,?
    WHERE EXISTS(SELECT 1 FROM triage_jobs WHERE id=?) ON CONFLICT(job_id,event_key) DO NOTHING`,
  [job.id, `deny:${step}:${ordinal}`, step, step === "read" ? "ticket.read" : `plugin.${step}`,
    ordinal, job.id, step, ordinal, outcome, Date.now(), job.id]);
}
