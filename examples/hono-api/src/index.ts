import { Hono } from "hono";

const app = new Hono();

app.get("/", (c) => c.json({ ok: true }));
app.get("/hello/:name", (c) => c.json({ hello: c.req.param("name") }));

export default app;
