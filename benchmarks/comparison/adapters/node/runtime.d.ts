declare module "node:http" {
  interface IncomingMessage {
    readonly url?: string;
  }

  interface ServerResponse {
    writeHead(statusCode: number, headers: Record<string, string>): this;
    end(body?: string | Uint8Array): void;
  }

  interface AddressInfo {
    readonly port: number;
  }

  interface Server {
    listen(port: number, hostname: string, callback: () => void): void;
    address(): AddressInfo | string | null;
  }

  export function createServer(
    listener: (request: IncomingMessage, response: ServerResponse) => void,
  ): Server;
}

declare const Buffer: {
  byteLength(value: string | Uint8Array): number;
};
