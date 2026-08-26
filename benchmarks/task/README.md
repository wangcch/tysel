# task

Run `tysel bench task`. The suite measures enqueue scaling at 100/1K/10K tasks,
claim/commit, queue claim, cancellation/deadline transitions, lease renewal,
crash requeue, and bounded-queue memory delta/backpressure behavior.

Task-throughput metrics remain observational. The 10,000-task bounded-queue
memory delta is gated at 32MiB to reject material storage regressions.
