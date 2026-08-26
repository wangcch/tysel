# Cron and Queue task worker

The application registers a UTC Cron heartbeat and one `jobs` Queue consumer.

Invoke the Queue handler directly:

```sh
tysel check
tysel queue jobs \
  --input '{"id":"job_123","action":"reindex"}' \
  --message-id msg_123
```

Run the scheduler with `tysel run`. For a quick Cron check, temporarily change
the expression to `* * * * *` and observe the next UTC minute boundary.

See the [Cron and Queue guide](../../docs/guides/cron-queue.md).
