# Run Cron and Queue handlers

This guide registers a UTC schedule and a named Queue consumer, invokes the
Queue path from the CLI, and verifies the scheduler without introducing a
second application protocol.

## Define both handlers

```ts
import type { TyselApp } from "@tysel/types";

interface Job {
  id: string;
  action: string;
}

export default {
  async fetch() {
    return Response.json({ queues: ["jobs"], cron: ["heartbeat"] });
  },
  tasks: {
    heartbeat: {
      kind: "cron",
      expression: "*/5 * * * *",
      async handler(context) {
        console.log(JSON.stringify({
          event: "heartbeat",
          requestId: context.requestId,
          deadlineMs: context.deadlineMs,
        }));
      },
    },
    processJob: {
      kind: "queue",
      name: "jobs",
      async handler(message: Job, context) {
        return {
          accepted: true,
          jobId: message.id,
          action: message.action,
          requestId: context.requestId,
        };
      },
    },
  },
} satisfies TyselApp;
```

The small HTTP handler keeps the same module runnable under `tysel run` while
the service-owned scheduler is active; Queue and Cron work still enters through
the task plane, not through this route.

The export key (`processJob`) names the handler. `name: "jobs"` names the Queue
address supplied to `tysel queue`. Only one consumer can register a given Queue
name in one application.

## Validate and invoke Queue

Use the complete [task worker example](https://github.com/wangcch/tysel/tree/main/examples/task-worker):

```sh
cd examples/task-worker
tysel check
tysel queue jobs \
  --input '{"id":"job_123","action":"reindex"}' \
  --message-id msg_123
```

The command prints the JSON result and exits non-zero for malformed input, an
unknown Queue, deadline expiry, or a handler failure. Message and result values
must be JSON serializable and are limited to 1 MiB each.

`--message-id` becomes the task idempotency key. Reuse it only when the producer
means the same logical message. Handler side effects still need application
idempotency; a key does not turn arbitrary external writes into exactly-once
operations.

## Run the scheduler

```sh
tysel run
```

Cron expressions have five fields and are evaluated in UTC:

```text
minute hour day-of-month month day-of-week
```

Numeric lists, ranges, and steps are supported. Month and weekday names are
not. Both `0` and `7` represent Sunday. When day-of-month and day-of-week are
both restricted, traditional Cron OR semantics apply.

For a quick verification, temporarily change the example expression to
`* * * * *`, start the service before the next minute boundary, and observe a
`heartbeat` log plus the runtime's `cron task queued` event. Restore the actual
production schedule afterward.

The local scheduler polls once per second, deduplicates one handler per UTC
minute, and can catch up at most the previous 24 hours. When the bounded broker
is full, it retains its cursor and admits due work incrementally as capacity
returns.

## Deadlines and failures

Queue and Cron tasks use `limits.request_timeout_ms`. `context.deadlineMs` is an
absolute Unix-millisecond deadline. Check it before starting an optional
upstream call, and give each dependency a shorter timeout.

Task handlers run on a bounded task worker path. A process restart does not
make an ordinary Queue or Cron handler durable. Use the [Durable API](../reference/runtime/durable.md)
when steps must be persisted, replayed, signaled, or resumed.

## Production checklist

- use UTC explicitly in operational schedules;
- make Queue and Cron side effects idempotent;
- give producers stable message IDs where retry identity matters;
- alert on handler failures, deadline expiry, and due-work delay;
- do not rely on more than 24 hours of local Cron catch-up;
- use an external durable producer when messages must survive host loss;
- keep task input and output below the documented protocol limits.

See the [application task types](../reference/runtime/application.md),
[`tysel queue`](../reference/cli/tasks.md#tysel-queue), and
[task limits](../reference/limits-and-defaults.md#application-task-and-mcp-bounds).
