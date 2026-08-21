# durable

Run `tysel bench durable`. The suite covers SQLite append, optional Postgres
append, suspend/resume, replay scaling, signal delivery, restart recovery, and
exactly-once effect replay behavior.

Set `TYSEL_DURABLE_POSTGRES_URL` (or `TYSEL_POSTGRES_TEST_URL`) for Postgres.
Without it, Postgres is reported as `skipped`, never as a fabricated zero. The
roadmap release gate applies to `resume_ms` (p50 ≤ 10ms).

SQLite and Postgres append samples both exclude store connection/setup and time
32 event appends. Replay samples prepare the recorded effect history before the
timer starts; restart recovery similarly prepares and closes the database before
timing reopen, history load, and cursor replay.
