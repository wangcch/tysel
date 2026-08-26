import type { TyselApp } from "@tysel/types";

interface TransformInput {
  items: string[];
}

export default {
  async fetch(request, runtime) {
    if (new URL(request.url).pathname !== "/transform") {
      return Response.json({ endpoint: "/transform" }, { status: 404 });
    }

    const source = await runtime.fs.read("input/jobs.json");
    const input = JSON.parse(source) as TransformInput;
    if (!Array.isArray(input.items) || input.items.some((item) => typeof item !== "string")) {
      return Response.json({ error: "items must be a string array" }, { status: 400 });
    }
    const result = {
      count: input.items.length,
      items: input.items.map((item) => item.toUpperCase()),
    };
    await runtime.fs.write("output/result.json", JSON.stringify(result, null, 2));
    return Response.json(result);
  },
} satisfies TyselApp;
