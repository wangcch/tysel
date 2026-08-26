import type { TyselApp } from "@tysel/types";

/** Postgres service. Requires `TYSEL_POSTGRES_MAIN`. */
export default {
  async fetch(_request, runtime) {
    await runtime.postgres.exec(
      "CREATE TABLE IF NOT EXISTS greetings (id INTEGER PRIMARY KEY, name TEXT NOT NULL)",
    );
    await runtime.postgres.exec(
      "INSERT INTO greetings (id, name) VALUES ($1, $2) ON CONFLICT (id) DO UPDATE SET name = EXCLUDED.name",
      [1, "tysel"],
    );
    const rows = await runtime.postgres.query(
      "SELECT id, name FROM greetings WHERE id = $1",
      [1],
    );
    return Response.json({ rows });
  },
} satisfies TyselApp;
