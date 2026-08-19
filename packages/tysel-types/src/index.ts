export type TrustMode = "trusted-service" | "isolated-task";

export type ExecutionProfile = "service" | "isolate";

export interface CapabilityRequirement {
  id: string;
  resources: string[];
}
