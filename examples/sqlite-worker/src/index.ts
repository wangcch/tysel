import type { TyselApp } from "@tysel/types";

/** SQLite-backed counter service. */
export default {
  async fetch(request, runtime) {
    await runtime.sqlite.exec(
      "CREATE TABLE IF NOT EXISTS kv (key TEXT PRIMARY KEY, value INTEGER NOT NULL)",
    );
    const url = new URL(request.url);
    const key = url.searchParams.get("key") ?? "hits";
    await runtime.sqlite.exec(
      "INSERT INTO kv(key, value) VALUES (?, 1) ON CONFLICT(key) DO UPDATE SET value = value + 1",
      [key],
    );
    const rows = await runtime.sqlite.query("SELECT value FROM kv WHERE key = ?", [key]);
    const value = typeof rows[0]?.value === "number" ? rows[0].value : 0;
    return Response.json({ key, value });
  },
} satisfies TyselApp;
