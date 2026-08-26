import type { TyselApp } from "@tysel/types";

export default {
  async fetch(request, runtime) {
    const url = new URL(request.url);
    if (request.method !== "POST" || url.pathname !== "/generate") {
      return Response.json({ endpoint: "POST /generate" }, { status: 404 });
    }

    const body = (await request.json()) as { prompt?: unknown };
    if (typeof body.prompt !== "string" || body.prompt.length === 0) {
      return Response.json({ error: "prompt must be a non-empty string" }, { status: 400 });
    }

    const result = await runtime.llm.generate({
      model: "default",
      system: "Return one concise sentence.",
      input: body.prompt,
      maxOutputTokens: 128,
      temperature: 0.2,
    });
    return Response.json(result);
  },
} satisfies TyselApp;
