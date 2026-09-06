/** Versioned, server-side Web API subset installed into each isolate. */
export const webApiVersion = "0.2.0";

export type WebEventListener<E extends TyselEvent = TyselEvent> =
  | ((event: E) => void)
  | { handleEvent(event: E): void };

export interface WebEventListenerOptions {
  capture?: boolean;
  once?: boolean;
  signal?: TyselAbortSignal;
}

export interface TyselEvent {
  readonly type: string;
  readonly target: TyselEventTarget | null;
  readonly currentTarget: TyselEventTarget | null;
  readonly bubbles: boolean;
  readonly cancelable: boolean;
  readonly composed: boolean;
  readonly defaultPrevented: boolean;
  readonly timeStamp: number;
  preventDefault(): void;
  stopPropagation(): void;
  stopImmediatePropagation(): void;
}

export interface TyselEventTarget {
  addEventListener(
    type: string,
    listener: WebEventListener,
    options?: boolean | WebEventListenerOptions,
  ): void;
  removeEventListener(
    type: string,
    listener: WebEventListener,
    options?: boolean | Pick<WebEventListenerOptions, "capture">,
  ): void;
  dispatchEvent(event: TyselEvent): boolean;
}

export interface TyselAbortSignal extends TyselEventTarget {
  readonly aborted: boolean;
  readonly reason: unknown;
  onabort: WebEventListener | null;
  throwIfAborted(): void;
}

export interface TyselAbortController {
  readonly signal: TyselAbortSignal;
  abort(reason?: unknown): void;
}

export interface TyselURLSearchParams extends Iterable<[string, string]> {
  readonly size: number;
  append(name: string, value: string): void;
  delete(name: string): void;
  get(name: string): string | null;
  getAll(name: string): string[];
  has(name: string): boolean;
  set(name: string, value: string): void;
  sort(): void;
  entries(): IterableIterator<[string, string]>;
  keys(): IterableIterator<string>;
  values(): IterableIterator<string>;
  forEach(
    callback: (value: string, key: string, parent: TyselURLSearchParams) => void,
    thisArg?: unknown,
  ): void;
  toString(): string;
}

export interface TyselURL {
  href: string;
  protocol: string;
  readonly origin: string;
  host: string;
  hostname: string;
  port: string;
  pathname: string;
  search: string;
  readonly searchParams: TyselURLSearchParams;
  hash: string;
  toString(): string;
  toJSON(): string;
}

export interface TyselHeaders extends Iterable<[string, string]> {
  append(name: string, value: string): void;
  delete(name: string): void;
  get(name: string): string | null;
  has(name: string): boolean;
  set(name: string, value: string): void;
  entries(): IterableIterator<[string, string]>;
  keys(): IterableIterator<string>;
  values(): IterableIterator<string>;
  forEach(
    callback: (value: string, key: string, parent: TyselHeaders) => void,
    thisArg?: unknown,
  ): void;
}

export type TyselHeadersInit =
  | TyselHeaders
  | Iterable<readonly [string, string]>
  | Record<string, string>;

export type TyselBodyInit = string | ArrayBuffer | ArrayBufferView;

export interface TyselRequestInit {
  method?: string;
  headers?: TyselHeadersInit;
  body?: TyselBodyInit | null;
  signal?: TyselAbortSignal | null;
}

export interface TyselResponseInit {
  status?: number;
  headers?: TyselHeadersInit;
}

export interface TyselBody {
  readonly bodyUsed: boolean;
  text(): Promise<string>;
  json(): Promise<unknown>;
  arrayBuffer(): Promise<ArrayBuffer>;
}

export interface TyselRequest extends TyselBody {
  readonly body: unknown | null;
  readonly url: string;
  readonly method: string;
  readonly headers: TyselHeaders;
  readonly signal: TyselAbortSignal | null;
  clone(): TyselRequest;
}

export interface TyselResponse extends TyselBody {
  readonly body: unknown | null;
  readonly status: number;
  readonly ok: boolean;
  readonly headers: TyselHeaders;
  clone(): TyselResponse;
}

export interface TyselCryptoKey {
  readonly type: "secret";
  readonly extractable: boolean;
  readonly algorithm: Readonly<{
    name: "HMAC";
    hash: Readonly<{ name: string }>;
    length: number;
  }>;
  readonly usages: readonly ("sign" | "verify")[];
}

export interface TyselSubtleCrypto {
  digest(
    algorithm: string | { name: string },
    data: BufferSource,
  ): Promise<ArrayBuffer>;
  importKey(
    format: "raw",
    keyData: BufferSource,
    algorithm:
      | string
      | { name: string; hash?: string | { name: string }; length?: number },
    extractable: boolean,
    keyUsages: readonly ("sign" | "verify")[],
  ): Promise<TyselCryptoKey>;
  sign(
    algorithm: "HMAC" | { name: "HMAC" },
    key: TyselCryptoKey,
    data: BufferSource,
  ): Promise<ArrayBuffer>;
  verify(
    algorithm: "HMAC" | { name: "HMAC" },
    key: TyselCryptoKey,
    signature: BufferSource,
    data: BufferSource,
  ): Promise<boolean>;
}

export type TyselIntegerTypedArray =
  | Int8Array
  | Uint8Array
  | Uint8ClampedArray
  | Int16Array
  | Uint16Array
  | Int32Array
  | Uint32Array
  | BigInt64Array
  | BigUint64Array;

export interface TyselCrypto {
  getRandomValues<T extends TyselIntegerTypedArray>(typedArray: T): T;
  readonly subtle: TyselSubtleCrypto;
}

export interface TyselTextEncoder {
  readonly encoding: "utf-8";
  encode(input?: string): Uint8Array;
}

export interface TyselTextDecoder {
  readonly encoding: "utf-8";
  readonly fatal: boolean;
  readonly ignoreBOM: boolean;
  decode(input?: BufferSource): string;
}

export type TyselFetch = (
  input: string | TyselRequest,
  init?: TyselRequestInit,
) => Promise<TyselResponse>;

/** Constructor/function surface corresponding exactly to the supported subset. */
export interface TyselWebApiGlobals {
  Event: new (
    type: string,
    init?: { bubbles?: boolean; cancelable?: boolean; composed?: boolean },
  ) => TyselEvent;
  EventTarget: new () => TyselEventTarget;
  URL: new (url: string, base?: string | TyselURL) => TyselURL;
  URLSearchParams: new (
    init?: string | Iterable<readonly [string, string]> | Record<string, string>,
  ) => TyselURLSearchParams;
  Headers: new (init?: TyselHeadersInit) => TyselHeaders;
  Request: new (input: string | TyselRequest, init?: TyselRequestInit) => TyselRequest;
  Response: {
    new (body?: TyselBodyInit | readonly TyselBodyInit[] | null, init?: TyselResponseInit): TyselResponse;
    json(data: unknown, init?: TyselResponseInit): TyselResponse;
  };
  AbortController: new () => TyselAbortController;
  AbortSignal: {
    abort(reason?: unknown): TyselAbortSignal;
    timeout(milliseconds: number): TyselAbortSignal;
  };
  TextEncoder: new () => TyselTextEncoder;
  TextDecoder: new (
    label?: "utf-8" | "utf8",
    options?: { fatal?: boolean; ignoreBOM?: boolean },
  ) => TyselTextDecoder;
  crypto: TyselCrypto;
  fetch: TyselFetch;
  setTimeout(
    handler: (...args: unknown[]) => void,
    milliseconds?: number,
    ...args: unknown[]
  ): number;
  clearTimeout(id?: number): void;
  setInterval(
    handler: (...args: unknown[]) => void,
    milliseconds?: number,
    ...args: unknown[]
  ): number;
  clearInterval(id?: number): void;
}
