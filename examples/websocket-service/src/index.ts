import type {} from "@tysel/types";

export default {
  async fetch(request: Request): Promise<Response> {
    const url = new URL(request.url);
    const upgrade = request.headers.get("upgrade")?.toLowerCase();
    if (url.pathname !== "/ws" || upgrade !== "websocket") {
      return Response.json({ websocket: "/ws" }, { status: 426 });
    }

    const socket = tysel.acceptWebSocket();
    socket.addEventListener("message", async (event) => {
      await socket.send(`echo:${event.data}`);
    });
    return new Response(null, { status: 101 });
  },
};
