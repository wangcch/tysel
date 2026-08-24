import { createServer } from "node:http";

function bodyFor(path: string): [contentType: string, body: string | Uint8Array] | null {
  if (path === "/health") return ["text/plain", "ok"];
  if (path === "/json/1k") {
    return ["application/json", JSON.stringify({ payload: "a".repeat(1024) })];
  }
  if (path === "/json/64k") {
    return ["application/json", JSON.stringify({ payload: "b".repeat(65536) })];
  }
  if (path === "/bytes/64k") return ["application/octet-stream", "x".repeat(65536)];
  if (path === "/bytes/64k-typed") {
    const body = new Uint8Array(65536);
    body.fill(120);
    return ["application/octet-stream", body];
  }
  return null;
}

const server = createServer((request, response) => {
  const result = bodyFor(new URL(request.url ?? "/", "http://localhost").pathname);
  if (!result) {
    response.writeHead(404, { "content-type": "text/plain", "content-length": "9" });
    response.end("not found");
    return;
  }
  const [contentType, body] = result;
  response.writeHead(200, {
    "content-type": contentType,
    "content-length": String(Buffer.byteLength(body)),
  });
  response.end(body);
});

server.listen(0, "127.0.0.1", () => {
  const address = server.address();
  if (!address || typeof address === "string") {
    throw new Error("Node server did not expose a TCP address");
  }
  console.log(`comparison listen 127.0.0.1:${address.port}`);
});
