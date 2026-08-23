declare const Bun: {
  serve(options: {
    hostname: string;
    port: number;
    fetch(request: Request): Response | Promise<Response>;
  }): { readonly port: number };
};

declare const Deno: {
  serve(
    options: {
      hostname: string;
      port: number;
      onListen(address: { hostname: string; port: number }): void;
    },
    handler: (request: Request) => Response | Promise<Response>,
  ): unknown;
};
