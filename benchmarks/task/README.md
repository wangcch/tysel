# task

Run `tysel bench task`. The suite measures enqueue scaling at 100/1K/10K tasks,
claim/commit, queue claim, cancellation/deadline transitions, lease renewal,
crash requeue, and bounded-queue memory delta/backpressure behavior.

These are observational metrics; the roadmap does not define a task-throughput
release threshold yet.
