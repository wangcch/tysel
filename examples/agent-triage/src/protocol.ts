export type Ticket = { id: string; priority: number };
export type Snapshot = { customerId: string; tickets: Ticket[] };
export type Limits = { callMs: number; jobMs: number; perCustomer: number; active: number; retained: number };
export type Config = {
  bootId: string;
  dispatchToken: string;
  pluginOrigin: string;
  adapterOrigin: string;
  pluginDigest: string;
  callerDigest: string;
  adapterId: string;
  limits: Limits;
  customers: { id: string; token: string; tickets: Ticket[] }[];
};

export class Failure extends Error {
  constructor(readonly code: string, readonly status = 400) { super(code); }
}

export function object(value: unknown): value is Record<string, unknown> {
  return value !== null && typeof value === "object" && !Array.isArray(value);
}

export function fields(value: unknown, keys: string[]): value is Record<string, unknown> {
  return object(value) && Object.keys(value).length === keys.length && keys.every((key) => key in value);
}

export function text(value: unknown, limit: number): value is string {
  return typeof value === "string" && value.trim().length > 0 && value.length <= limit;
}

// Inspect object member names before normal parsing discards duplicate fields.
// Iterative scanning also bounds nesting before invoking the native JSON parser.
function strictJson(source: string): unknown {
  const scopes: (Set<string> | null)[] = [];
  for (let at = 0; at < source.length; at++) {
    const char = source[at];
    if (char === "{") scopes.push(new Set());
    else if (char === "[") scopes.push(null);
    else if (char === "}" || char === "]") scopes.pop();
    else if (char === '"') {
      const start = at;
      for (at++; at < source.length; at++) {
        if (source[at] === "\\") at++;
        else if (source[at] === '"') break;
      }
      let next = at + 1;
      while (/^[ \t\r\n]$/.test(source[next] ?? "")) next++;
      const members = scopes[scopes.length - 1];
      if (source[next] === ":" && members) {
        const name = JSON.parse(source.slice(start, at + 1)) as string;
        if (members.has(name)) throw new Failure("INVALID_JSON");
        members.add(name);
      }
    }
    if (scopes.length > 32) throw new Failure("INVALID_JSON");
  }
  return JSON.parse(source);
}

// Bound bytes before parsing, including when Content-Length is absent or false.
export async function jsonBody(message: Request | Response, limit: number): Promise<unknown> {
  const reader = message.body?.getReader();
  if (!reader) throw new Failure("INVALID_JSON");
  const chunks: Uint8Array[] = [];
  let size = 0;
  try {
    for (;;) {
      const next = await reader.read();
      if (next.done) break;
      size += next.value.byteLength;
      if (size > limit) throw new Failure("MESSAGE_TOO_LARGE", 413);
      chunks.push(next.value);
    }
  } catch (error) {
    await reader.cancel().catch(() => {});
    throw error;
  } finally {
    reader.releaseLock();
  }
  const body = new Uint8Array(size);
  let at = 0;
  for (const chunk of chunks) { body.set(chunk, at); at += chunk.byteLength; }
  try { return strictJson(new TextDecoder("utf-8", { fatal: true }).decode(body)); }
  catch { throw new Failure("INVALID_JSON"); }
}

export async function hash(value: string): Promise<string> {
  const bytes = await crypto.subtle.digest("SHA-256", new TextEncoder().encode(value));
  return Array.from(new Uint8Array(bytes), (byte) => byte.toString(16).padStart(2, "0")).join("");
}

export function ticketIds(value: unknown): string[] {
  if (!fields(value, ["protocolVersion", "ticketIds"]) || value.protocolVersion !== 1 ||
      !Array.isArray(value.ticketIds) || value.ticketIds.length > 16) throw new Failure("INVALID_INPUT");
  const seen = new Set<string>();
  return value.ticketIds.map((id: unknown) => {
    if (!text(id, 128) || seen.has(id)) throw new Failure("INVALID_INPUT");
    seen.add(id);
    return id;
  });
}

export function admission(ids: string[], customer: Config["customers"][number]): Snapshot {
  const tickets = ids.map((id) => {
    const ticket = customer.tickets.find((ticket) => ticket.id === id);
    if (!ticket) throw new Failure("FORBIDDEN_TICKET", 403);
    return { id: ticket.id, priority: ticket.priority };
  });
  return { customerId: customer.id, tickets };
}
