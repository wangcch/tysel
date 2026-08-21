import type {} from "@tysel/types";

export default {
  async fetch(): Promise<Response> {
    return Response.json({ tool: "lookup", isolated: true });
  },
  tasks: {
    lookup: {
      kind: "mcp" as const,
      description: "Look up a customer without host I/O",
      input: { customerId: "string" },
      async handler(input: { customerId: string }) {
        return {
          customerId: input.customerId,
          secret: await tysel.secrets.ref("OPENAI_API_KEY"),
          isolated: true,
        };
      },
    },
  },
};
