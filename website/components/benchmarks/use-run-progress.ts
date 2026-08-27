"use client";

import { useEffect, useState } from "react";

function easeOutCubic(t: number): number {
  return 1 - (1 - t) ** 3;
}

function usePrefersReducedMotion(): boolean {
  const [reduced, setReduced] = useState(false);

  useEffect(() => {
    const media = window.matchMedia("(prefers-reduced-motion: reduce)");
    const sync = () => setReduced(media.matches);
    sync();
    media.addEventListener("change", sync);
    return () => media.removeEventListener("change", sync);
  }, []);

  return reduced;
}

/**
 * Count-up from 0 → 1 after mount.
 * Starts at 0 on SSR and client so we never flash the final value then snap back.
 * Uses timers (not only rAF) so background / embedded tabs still reach 1.
 */
export function useRunProgress(delayMs: number, durationMs: number): number {
  const reduced = usePrefersReducedMotion();
  const [progress, setProgress] = useState(0);

  useEffect(() => {
    if (reduced) {
      setProgress(1);
      return;
    }

    let cancelled = false;
    const origin = performance.now();

    const paint = () => {
      if (cancelled) return;
      const elapsed = performance.now() - origin - delayMs;
      if (elapsed < 0) return;
      const next = easeOutCubic(Math.min(1, elapsed / durationMs));
      setProgress(next);
    };

    const interval = window.setInterval(paint, 32);
    const hardStop = window.setTimeout(() => {
      if (!cancelled) setProgress(1);
      window.clearInterval(interval);
    }, delayMs + durationMs + 50);

    // Kick once in case the first interval is delayed.
    paint();

    return () => {
      cancelled = true;
      window.clearInterval(interval);
      window.clearTimeout(hardStop);
    };
  }, [delayMs, durationMs, reduced]);

  return progress;
}
