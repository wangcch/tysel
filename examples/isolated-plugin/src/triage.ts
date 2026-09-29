// Application protocol for one bounded lookup, not a runtime capability API.
function text(value: unknown, limit: number): value is string {
  return typeof value === "string" && value.trim().length > 0 && value.length <= limit;
}

function object(value: unknown): value is Record<string, unknown> {
  return value !== null && typeof value === "object" && !Array.isArray(value);
}

export async function triage(request: Request): Promise<Response> {
  if (request.method !== "POST") {
    return Response.json({ error: "use POST" }, { status: 405, headers: { Allow: "POST" } });
  }
  const invalid = () => Response.json({ error: "invalid triage snapshot" }, { status: 400 });
  let input: unknown;
  try {
    input = await request.json();
  } catch {
    return invalid();
  }
  if (!object(input) || !text(input.customerId, 128) || !Array.isArray(input.tickets) ||
      input.tickets.length > 16 ||
      Object.keys(input).some((key) => !["customerId", "tickets", "detail"].includes(key))) {
    return invalid();
  }
  const tickets: { id: string; priority: number }[] = [];
  for (const item of input.tickets) {
    if (!object(item) || !text(item.id, 128) || typeof item.priority !== "number" ||
        !Number.isInteger(item.priority) || item.priority < 0 || item.priority > 3 ||
        Object.keys(item).some((key) => !["id", "priority"].includes(key)) ||
        tickets.some((ticket) => ticket.id === item.id)) {
      return invalid();
    }
    tickets.push({ id: item.id, priority: item.priority });
  }
  // Stable ties: preserve the caller's order. Larger priority means more urgent.
  tickets.sort((a, b) => b.priority - a.priority);
  const selected = tickets[0];
  if (!selected) {
    if (input.detail !== undefined) return invalid();
    return Response.json({ kind: "done", customerId: input.customerId, summary: "No open tickets." });
  }
  if (input.detail === undefined) {
    return Response.json({ kind: "lookup", operation: "ticket.read", ticketId: selected.id });
  }
  const detail = input.detail;
  if (!object(detail) || detail.ticketId !== selected.id || !text(detail.subject, 500) ||
      Object.keys(detail).some((key) => !["ticketId", "subject"].includes(key))) {
    return invalid();
  }
  return Response.json({
    kind: "done",
    customerId: input.customerId,
    summary: `Prioritize ${selected.id}: ${detail.subject}`,
  });
}

// Versioned application envelope for the trusted caller example. Identifiers
// correlate a response; the caller still independently validates its authority.
export async function triageEnvelope(request: Request): Promise<Response> {
  if (request.method !== "POST") {
    return Response.json({ error: "use POST" }, { status: 405, headers: { Allow: "POST" } });
  }
  let input: unknown;
  try { input = await request.json(); } catch {
    return Response.json({ error: "invalid envelope" }, { status: 400 });
  }
  if (!object(input) || input.protocolVersion !== 1 || !text(input.jobId, 64) ||
      !["1", "2"].includes(String(input.stepId)) || typeof input.stepId !== "string" ||
      !text(input.attemptId, 64) || !object(input.payload) ||
      Object.keys(input).length !== 5) {
    return Response.json({ error: "invalid envelope" }, { status: 400 });
  }
  const response = await triage(new Request(request.url, {
    method: "POST", body: JSON.stringify(input.payload),
  }));
  if (!response.ok) return response;
  return Response.json({
    protocolVersion: 1, jobId: input.jobId, stepId: input.stepId,
    attemptId: input.attemptId, payload: await response.json(),
  });
}
