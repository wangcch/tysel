import type { TyselApp } from "@tysel/types";

interface AgentInput {
  runId: string;
  customerId: string;
  prompt?: string;
}

interface Approval {
  approved: boolean;
}

interface DurableContext {
  step<T>(name: string, fn: () => Promise<T> | T): Promise<T>;
  effect<T>(name: string, fn: () => Promise<T> | T): Promise<T>;
  waitForSignal<T>(name: string): Promise<T>;
  now(): Date;
}

interface RunRow {
  run_id: string;
  task_id: string | null;
  customer_id: string;
  status: string;
  draft_json: string | null;
  result_json: string | null;
  save_count: number;
  updated_at: string;
}

const CREATE_RUNS = `
  CREATE TABLE IF NOT EXISTS durable_agent_runs (
    run_id TEXT PRIMARY KEY,
    task_id TEXT UNIQUE,
    customer_id TEXT NOT NULL,
    status TEXT NOT NULL,
    draft_json TEXT,
    result_json TEXT,
    save_count INTEGER NOT NULL DEFAULT 0,
    updated_at TEXT NOT NULL
  )
`;

async function ensureRuns(): Promise<void> {
  await tysel.sqlite.exec(CREATE_RUNS);
}

function newRunId(): string {
  const bytes = crypto.getRandomValues(new Uint8Array(16));
  return Array.from(bytes, (byte) => byte.toString(16).padStart(2, "0")).join("");
}

function parseJson(value: string | null): unknown {
  return value === null ? null : JSON.parse(value);
}

function publicRun(row: RunRow): Record<string, unknown> {
  return {
    runId: row.run_id,
    taskId: row.task_id,
    customerId: row.customer_id,
    status: row.status,
    draft: parseJson(row.draft_json),
    result: parseJson(row.result_json),
    saveCount: row.save_count,
    updatedAt: row.updated_at,
  };
}

async function findRun(runId: string): Promise<RunRow | null> {
  await ensureRuns();
  const rows = await tysel.sqlite.query(
    `SELECT run_id, task_id, customer_id, status, draft_json, result_json,
            save_count, updated_at
       FROM durable_agent_runs WHERE run_id = ?`,
    [runId],
  );
  return (rows[0] as unknown as RunRow | undefined) ?? null;
}

const agent = async (ctx: DurableContext, input: AgentInput) => {
  const customer = await ctx.step("load-customer", async () => ({
    id: input.customerId,
    name: `Customer ${input.customerId}`,
  }));
  const draft = await ctx.effect("draft-with-llm", () =>
    tysel.llm.generate({
      model: "default",
      input: input.prompt ?? `Summarize account ${customer.id}`,
      system: "You are a concise account assistant.",
    }),
  );
  const draftedAt = ctx.now().toISOString();
  await ctx.effect("persist-draft", async () => {
    await ensureRuns();
    await tysel.sqlite.exec(
      `INSERT INTO durable_agent_runs
         (run_id, customer_id, status, draft_json, updated_at)
       VALUES (?, ?, 'awaiting_approval', ?, ?)
       ON CONFLICT(run_id) DO UPDATE SET
         status = 'awaiting_approval',
         draft_json = excluded.draft_json,
         updated_at = excluded.updated_at`,
      [input.runId, input.customerId, JSON.stringify(draft), draftedAt],
    );
    return true;
  });

  const approval = await ctx.waitForSignal<Approval>("approval");
  const result = {
    customer,
    draft,
    approved: approval.approved === true,
    savedAt: ctx.now().toISOString(),
  };
  await ctx.effect("save-result", async () => {
    await tysel.sqlite.exec(
      `UPDATE durable_agent_runs
          SET status = ?, result_json = ?, save_count = save_count + 1, updated_at = ?
        WHERE run_id = ?`,
      [
        result.approved ? "completed" : "rejected",
        JSON.stringify(result),
        result.savedAt,
        input.runId,
      ],
    );
    return result;
  });
  return result;
};

export default {
  async fetch(request, runtime) {
    const url = new URL(request.url);
    if (request.method === "POST" && url.pathname === "/runs") {
      const body = (await request.json()) as { customerId?: unknown; prompt?: unknown };
      if (typeof body.customerId !== "string" || body.customerId.length === 0) {
        return Response.json({ error: "customerId must be a non-empty string" }, { status: 400 });
      }
      const input: AgentInput = {
        runId: newRunId(),
        customerId: body.customerId,
        ...(typeof body.prompt === "string" ? { prompt: body.prompt } : {}),
      };
      await ensureRuns();
      const started = runtime.durable.start("agent", input);
      await tysel.sqlite.exec(
        `UPDATE durable_agent_runs SET task_id = ?, updated_at = ? WHERE run_id = ?`,
        [started.taskId, new Date().toISOString(), input.runId],
      );
      const run = await findRun(input.runId);
      return Response.json(run === null ? started : publicRun(run), { status: 202 });
    }

    const runRoute = url.pathname.match(/^\/runs\/([^/]+)$/);
    if (request.method === "GET" && runRoute) {
      const runId = decodeURIComponent(runRoute[1]!);
      const run = await findRun(runId);
      return run === null
        ? Response.json({ error: "run not found" }, { status: 404 })
        : Response.json(publicRun(run));
    }

    const approvalRoute = url.pathname.match(/^\/runs\/([^/]+)\/approval$/);
    if (request.method === "POST" && approvalRoute) {
      const runId = decodeURIComponent(approvalRoute[1]!);
      const run = await findRun(runId);
      if (run === null || run.task_id === null) {
        return Response.json({ error: "run not found" }, { status: 404 });
      }
      const body = (await request.json()) as { approved?: unknown };
      if (typeof body.approved !== "boolean") {
        return Response.json({ error: "approved must be a boolean" }, { status: 400 });
      }
      runtime.durable.sendSignal(run.task_id, "approval", { approved: body.approved });
      return Response.json({ runId, status: "approval_queued" }, { status: 202 });
    }

    return Response.json({
      message: "Durable Agent Golden Path",
      start: "POST /runs { customerId, prompt? }",
      status: "GET /runs/:runId",
      approve: "POST /runs/:runId/approval { approved: true }",
    });
  },
  durable: { agent },
} satisfies TyselApp;
