import type { TyselApp } from "@tysel/types";

export default {
  async fetch(request) {
    return Response.json({
      message: "Hello from Tysel",
      path: new URL(request.url).pathname,
    });
  },
} satisfies TyselApp;
