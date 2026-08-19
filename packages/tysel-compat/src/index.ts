export type CompatTier = "A" | "B" | "C" | "D" | "E";

export interface CompatFinding {
  name: string;
  tier: CompatTier;
  reason?: string;
}

export const shimAllowlist = [
  "buffer",
  "path",
  "util",
  "events",
  "assert",
  "querystring",
] as const;
