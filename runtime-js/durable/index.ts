/** Replay-safe durable primitives. Persistence is implemented in `tysel-durable`. */
export interface DurableRetryPolicy {
  maxAttempts?: number;
  delay?: string | number;
  factor?: number;
  maxDelay?: string | number;
}

export interface DurableContext {
  step<T>(name: string, fn: () => Promise<T> | T): Promise<T>;
  effect<T>(name: string, fn: () => Promise<T> | T): Promise<T>;
  sleep(duration: string | number): Promise<void>;
  waitForSignal<T = unknown>(name: string): Promise<T>;
  retry<T>(
    policy: DurableRetryPolicy,
    fn: (attempt: number) => Promise<T> | T,
  ): Promise<T>;
  now(): Date;
  random(): number;
}

/** @deprecated Use `DurableContext`; retained for source compatibility. */
export type DurableHost = DurableContext;
