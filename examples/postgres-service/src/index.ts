import type {} from "@tysel/types";

/** Postgres service demo (M2). Requires `TYSEL_POSTGRES_MAIN`. */
export default {
  async fetch(): Promise<Response> {
    await tysel.postgres.exec(
      "CREATE TABLE IF NOT EXISTS greetings (id INTEGER PRIMARY KEY, name TEXT NOT NULL)",
    );
    await tysel.postgres.exec(
      "INSERT INTO greetings (id, name) VALUES ($1, $2) ON CONFLICT (id) DO UPDATE SET name = EXCLUDED.name",
      [1, "tysel"],
    );
    const rows = await tysel.postgres.query(
      "SELECT id, name FROM greetings WHERE id = $1",
      [1],
    );
    return Response.json({ rows });
  },
};
