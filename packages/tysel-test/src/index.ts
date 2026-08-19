export async function invokeFetch(
  handler: (request: Request) => Response | Promise<Response>,
  input: RequestInfo | URL,
  init?: RequestInit,
): Promise<Response> {
  const request = input instanceof Request ? input : new Request(input, init);
  return handler(request);
}
