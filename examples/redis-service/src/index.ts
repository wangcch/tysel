import type { TyselApp } from "@tysel/types";

/** Redis service. Requires `TYSEL_REDIS_CACHE`. */
export default {
  async fetch(request, runtime) {
    const key = new URL(request.url).searchParams.get("key") ?? "greeting";
    let value = await runtime.redis.get(key);
    if (value === null) {
      value = "hello from tysel";
      await runtime.redis.set(key, value, { ttlSeconds: 60 });
    }
    return Response.json({ key, value, cached: await runtime.redis.exists(key) });
  },
} satisfies TyselApp;
