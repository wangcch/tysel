function responseFor(path: string): Response {
  if (path === "/health") return new Response("ok");
  if (path === "/json/1k") return Response.json({ payload: "a".repeat(1024) });
  if (path === "/json/64k") return Response.json({ payload: "b".repeat(65536) });
  if (path === "/bytes/64k") {
    return new Response("x".repeat(65536), {
      headers: { "content-type": "application/octet-stream" },
    });
  }
  if (path === "/bytes/64k-typed") {
    const body = new Uint8Array(65536);
    body.fill(120);
    return new Response(body, {
      headers: { "content-type": "application/octet-stream" },
    });
  }
  return new Response("not found", { status: 404 });
}

const server = Bun.serve({
  hostname: "127.0.0.1",
  port: 0,
  fetch(request) {
    return responseFor(new URL(request.url).pathname);
  },
});

console.log(`comparison listen 127.0.0.1:${server.port}`);

export {};
