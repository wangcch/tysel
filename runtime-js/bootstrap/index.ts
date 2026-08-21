/** Build-time manifest for the JavaScript layers embedded by `tysel-engine-qjs`. */
export const runtimeJsVersion = "0.0.1";

export const runtimeLayers = ["web-api", "capability-client", "durable"] as const;

/**
 * The native host installs the embedded layers before application evaluation.
 * This function is intentionally side-effect free for tooling and contract tests.
 */
export function bootstrap(): void {}
