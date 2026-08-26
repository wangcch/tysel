import type { TyselApp } from "@tysel/types";

export default {
  async fetch(request, runtime) {
    const url = new URL(request.url);
    const upgrade = request.headers.get("upgrade")?.toLowerCase();
    if (url.pathname !== "/ws" || upgrade !== "websocket") {
      return Response.json({ websocket: "/ws" }, { status: 426 });
    }

    const socket = runtime.acceptWebSocket();
    socket.addEventListener("message", async (event) => {
      await socket.send(`echo:${event.data}`);
    });
    return new Response(null, { status: 101 });
  },
} satisfies TyselApp;
