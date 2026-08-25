"use client";

import { useEffect, useState } from "react";

const frames = [
  {
    id: "source",
    label: "src/index.ts",
    hint: "A Fetch handler is the whole application.",
    code: `export default {
  async fetch(request: Request): Promise<Response> {
    return Response.json({
      message: "Hello from Tysel",
      path: new URL(request.url).pathname,
    });
  },
};`,
  },
  {
    id: "build",
    label: "tysel build --release",
    hint: "Package the app and native runtime together.",
    code: `$ tysel build --release
bundled src/index.ts
validated tysel.toml
wrote dist/hello-tysel
wrote dist/hello-tysel.sha256
wrote dist/compat.json
wrote dist/sbom.json
wrote dist/evidence.json`,
  },
  {
    id: "run",
    label: "./dist/hello-tysel",
    hint: "No Node.js, V8, or node_modules on the target.",
    code: `$ ./dist/hello-tysel
tysel listen 127.0.0.1:3000

$ curl http://127.0.0.1:3000/hello
{"message":"Hello from Tysel","path":"/hello"}`,
  },
];

export function ArtifactDemo() {
  const [index, setIndex] = useState(0);
  const [paused, setPaused] = useState(false);
  const frame = frames[index];

  useEffect(() => {
    if (paused) return;
    const id = window.setInterval(() => {
      setIndex((current) => (current + 1) % frames.length);
    }, 4200);
    return () => window.clearInterval(id);
  }, [paused]);

  return (
    <div
      className="home-terminal overflow-hidden border border-fd-border bg-tysel-ink text-tysel-white"
      onMouseEnter={() => setPaused(true)}
      onMouseLeave={() => setPaused(false)}
    >
      <div className="flex items-center justify-between border-b border-white/10 px-3 py-2">
        <div className="flex flex-wrap gap-1">
          {frames.map((item, i) => (
            <button
              key={item.id}
              type="button"
              onClick={() => setIndex(i)}
              className={`px-2.5 py-1 text-xs font-medium transition-colors ${
                i === index
                  ? "bg-tysel-lime text-tysel-ink"
                  : "text-white/55 hover:text-white"
              }`}
            >
              {String(i + 1).padStart(2, "0")} {item.label}
            </button>
          ))}
        </div>
        <span className="hidden font-mono text-[10px] tracking-wide text-white/40 sm:inline">
          {paused ? "paused" : "replay"}
        </span>
      </div>
      <div className="border-b border-white/10 px-4 py-2">
        <p className="text-xs text-white/55">{frame.hint}</p>
      </div>
      <pre
        key={frame.id}
        className="home-code-fade min-h-[260px] overflow-x-auto p-4 font-mono text-[13px] leading-6 text-white/90"
      >
        <code>{frame.code}</code>
      </pre>
    </div>
  );
}
