/** Replay-safe durable primitives. Persistence is implemented in `tysel-durable`. */
export interface DurableHost {
  step<T>(name: string, fn: () => Promise<T> | T): Promise<T>;
  effect<T>(name: string, fn: () => Promise<T> | T): Promise<T>;
  sleep(duration: string | number): Promise<void>;
  waitForSignal<T = unknown>(name: string): Promise<T>;
  now(): Date;
  random(): number;
}
