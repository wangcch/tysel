import type { TyselWebApiGlobals } from "./index.js";
// Compile-only integration examples: explicit subset types must compose.
export function checkStreamTypes(api: TyselWebApiGlobals) {
  const signal = api.AbortSignal.any([new api.AbortController().signal]);
  const stream = new api.ReadableStream<Uint8Array>({
    start(controller) { controller.enqueue(new Uint8Array([65])); controller.close(); },
  });
  const decoder = new api.TextDecoderStream();
  const text = stream.pipeThrough(decoder);
  const done = text.pipeTo(new api.WritableStream<string>({ write(value) { void value; } }), { signal });
  const response = new api.Response(new api.ReadableStream<Uint8Array>());
  const cookie: string[] = response.headers.getSetCookie();
  const id: string = api.crypto.randomUUID();
  const encoded = new api.TextEncoder().encodeInto("hello", new Uint8Array(8));
  const redirect = api.Response.redirect("https://example.com/", 307);
  const errorType: "default" | "error" = api.Response.error().type;
  void [encoded.read, encoded.written, redirect, errorType];
  return { done, cookie, id };
}
