import type { TyselRuntime } from "@tysel/runtime-js/capability-client";

export type {
  AcceptedWebSocket,
  AcceptedWebSocketEvent,
  CapabilityHandle,
  DurableControlClient,
  DurableStartResult,
  FileSystemClient,
  LlmClient,
  SecretClient,
  SqlClient,
  TyselRuntime,
  WebSocketClient,
  WebSocketClientEvent,
} from "@tysel/runtime-js/capability-client";
export type {
  DurableContext,
  DurableHost,
  DurableRetryPolicy,
} from "@tysel/runtime-js/durable";
export type {
  TyselAbortController,
  TyselAbortSignal,
  TyselBody,
  TyselCrypto,
  TyselCryptoKey,
  TyselEvent,
  TyselEventTarget,
  TyselFetch,
  TyselHeaders,
  TyselHeadersInit,
  TyselIntegerTypedArray,
  TyselRequest,
  TyselRequestInit,
  TyselResponse,
  TyselResponseInit,
  TyselSubtleCrypto,
  TyselTextDecoder,
  TyselTextEncoder,
  TyselURL,
  TyselURLSearchParams,
  TyselWebApiGlobals,
  WebEventListener,
  WebEventListenerOptions,
} from "@tysel/runtime-js/web-api";

export type TrustMode = "trusted-service" | "isolated-task";

export type ExecutionProfile = "service" | "isolate";

export interface CapabilityRequirement {
  id: string;
  resources: string[];
}

/** Alias retained as the conventional application-facing host type. */
export type Tysel = TyselRuntime;

declare global {
  const tysel: TyselRuntime;
}
