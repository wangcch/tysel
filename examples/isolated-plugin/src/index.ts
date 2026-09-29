import type { TyselApp } from "@tysel/types";
import { triage, triageEnvelope } from "./triage.js";

type Probe = {
  capability: "fetch" | "filesystem";
  denied: boolean;
  error?: string;
};

async function probe(
  capability: Probe["capability"],
  operation: () => Promise<unknown>,
): Promise<Response> {
  try {
    await operation();
    return Response.json({ capability, denied: false } satisfies Probe, { status: 500 });
  } catch (error) {
    return Response.json(
      { capability, denied: true, error: String(error) } satisfies Probe,
      { status: 403 },
    );
  }
}

async function summarize(request: Request): Promise<Response> {
  if (request.method !== "POST") {
    return Response.json({ error: "use POST" }, { status: 405, headers: { Allow: "POST" } });
  }
  let input: unknown;
  try {
    input = await request.json();
  } catch {
    return Response.json({ error: "body must be valid JSON" }, { status: 400 });
  }
  if (input === null || typeof input !== "object" || Array.isArray(input)) {
    return Response.json({ error: "customer snapshot must be an object" }, { status: 400 });
  }
  const snapshot = input as Record<string, unknown>;
  const { customerId, name, openTickets } = snapshot;
  if (
    Object.keys(snapshot).some((key) => !["customerId", "name", "openTickets"].includes(key)) ||
    typeof customerId !== "string" || customerId.trim().length === 0 || customerId.length > 128 ||
    typeof name !== "string" || name.trim().length === 0 || name.length > 80 ||
    typeof openTickets !== "number" || !Number.isSafeInteger(openTickets) ||
    openTickets < 0 || openTickets > 10000
  ) {
    return Response.json(
      { error: "provide only customerId (1–128 characters), name (1–80 characters), and openTickets (integer 0–10000)" },
      { status: 400 },
    );
  }
  return Response.json({
    customerId,
    summary: `${name} has ${openTickets} open support ticket${openTickets === 1 ? "" : "s"}.`,
    needsAttention: openTickets > 0,
  });
}

export default {
  async fetch(request, runtime) {
    switch (new URL(request.url).pathname) {
      case "/triage/v1":
        return triageEnvelope(request);
      case "/triage":
        return triage(request);
      case "/summarize":
        return summarize(request);
      case "/probe/fetch":
        return probe("fetch", () => fetch("https://api.example.com/"));
      case "/probe/filesystem":
        return probe("filesystem", () => runtime.fs.read("data/example.txt"));
      default:
        return Response.json({ isolated: true, plugin: "echo" });
    }
  },
} satisfies TyselApp;
