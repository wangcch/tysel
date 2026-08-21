import type { TyselEvent, TyselEventTarget, TyselResponse } from "../web-api/index.ts";

/** Typed client for `tysel:*` capability imports. Host implements the ABI. */
export type CapabilityHandle = string;

export interface SqlClient {
  exec(sql: string, params?: readonly unknown[]): Promise<number>;
  query(sql: string, params?: readonly unknown[]): Promise<Record<string, unknown>[]>;
}

export interface FileSystemClient {
  read(path: string): Promise<string>;
  write(path: string, data: string): Promise<void>;
}

export interface SecretClient {
  ref(name: string): Promise<CapabilityHandle>;
}

export interface LlmClient {
  generate<T = unknown>(options: Record<string, unknown>): Promise<T>;
}

export interface AcceptedWebSocket extends TyselEventTarget {
  readonly readyState: number;
  onmessage: ((event: AcceptedWebSocketEvent) => void) | null;
  onclose: ((event: AcceptedWebSocketEvent) => void) | null;
  onerror: ((event: AcceptedWebSocketEvent) => void) | null;
  send(data: string): Promise<void>;
  close(): Promise<void>;
  addEventListener(
    type: "message" | "close" | "error",
    listener: ((event: AcceptedWebSocketEvent) => void) | { handleEvent(event: AcceptedWebSocketEvent): void },
    options?: { once?: boolean },
  ): void;
  removeEventListener(type: "message" | "close" | "error", listener: unknown): void;
}

export interface AcceptedWebSocketEvent extends TyselEvent {
  readonly type: string;
  readonly target: AcceptedWebSocket;
  readonly currentTarget: AcceptedWebSocket;
  readonly data?: string;
  readonly code?: number;
  readonly reason?: string;
  readonly wasClean?: boolean;
  readonly error?: unknown;
}

export interface WebSocketClient extends TyselEventTarget {
  readonly CONNECTING: 0;
  readonly OPEN: 1;
  readonly CLOSING: 2;
  readonly CLOSED: 3;
  readonly url: string;
  readonly opened: Promise<WebSocketClient>;
  readyState: number;
  binaryType: "arraybuffer";
  onopen: ((event: WebSocketClientEvent) => void) | null;
  onmessage: ((event: WebSocketClientEvent) => void) | null;
  onerror: ((event: WebSocketClientEvent) => void) | null;
  onclose: ((event: WebSocketClientEvent) => void) | null;
  send(data: string): Promise<void>;
  close(): Promise<void>;
}

export interface WebSocketClientEvent extends TyselEvent {
  readonly target: WebSocketClient;
  readonly data?: string | ArrayBuffer;
  readonly code?: number;
  readonly reason?: string;
  readonly wasClean?: boolean;
  readonly error?: unknown;
}

export interface DurableStartResult {
  status: string;
  taskId: string;
}

export interface DurableControlClient {
  start(name: string, input?: unknown): DurableStartResult;
  sendSignal(taskId: string, name: string, payload?: unknown): void;
}

/** Public JavaScript surface installed by the capability-client layer. */
export interface TyselRuntime {
  readonly isolateId: number;
  sleep(milliseconds: number): Promise<void>;
  echo(value: string): Promise<string>;
  httpGet(url: string): Promise<TyselResponse>;
  acceptWebSocket(): AcceptedWebSocket;
  sqlite: SqlClient;
  postgres: SqlClient;
  fs: FileSystemClient;
  secrets: SecretClient;
  llm: LlmClient;
  durable: DurableControlClient;
}
