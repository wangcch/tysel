import type { TyselApp } from "@tysel/types";

type Probe = {
  capability: "fetch" | "filesystem";
  denied: boolean;
  error?: string;
};

async function probe(
  capability: Probe["capability"],
  operation: () => Promise<unknown>,
): Promise<Response> {
  try {
    await operation();
    return Response.json({ capability, denied: false } satisfies Probe, { status: 500 });
  } catch (error) {
    return Response.json(
      { capability, denied: true, error: String(error) } satisfies Probe,
      { status: 403 },
    );
  }
}

export default {
  async fetch(request, runtime) {
    switch (new URL(request.url).pathname) {
      case "/probe/fetch":
        return probe("fetch", () => fetch("https://api.example.com/"));
      case "/probe/filesystem":
        return probe("filesystem", () => runtime.fs.read("data/example.txt"));
      default:
        return Response.json({ isolated: true, plugin: "echo" });
    }
  },
} satisfies TyselApp;
