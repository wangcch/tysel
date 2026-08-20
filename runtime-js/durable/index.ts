/** Replay-safe durable primitives. Persistence is implemented in `tysel-durable`. */
export interface DurableHost {
  step<T>(name: string, fn: () => Promise<T> | T): Promise<T>;
  effect<T>(name: string, fn: () => Promise<T> | T): Promise<T>;
  sleep(duration: string | number): Promise<void>;
  now(): Date;
  random(): number;
}
