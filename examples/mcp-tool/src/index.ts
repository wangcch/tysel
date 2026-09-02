import { defineApp } from "@tysel/sdk";

export default defineApp({
  async fetch() {
    return Response.json({ tool: "lookup", isolated: true });
  },
  tasks: {
    lookup: {
      kind: "mcp",
      description: "Look up a customer without host I/O",
      input: { customerId: "string" },
      async handler(input) {
        return {
          customerId: input.customerId,
          secret: await tysel.secrets.ref("OPENAI_API_KEY"),
          isolated: true,
        };
      },
    },
  },
});
