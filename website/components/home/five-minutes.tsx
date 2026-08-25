"use client";

import { useEffect, useState } from "react";

const steps = [
  {
    n: "01",
    title: "Verify the toolchain",
    command: "tysel doctor --install",
    copy: "Confirm tysel, tysel-service, and tysel-worker are on PATH together.",
  },
  {
    n: "02",
    title: "Create a project",
    command: "tysel init hello-tysel --yes",
    copy: "Writes src/index.ts, tests, and tysel.toml without overwriting existing files.",
  },
  {
    n: "03",
    title: "Validate configuration",
    command: "tysel config validate",
    copy: "The manifest is the contract: entry, profile, permissions, and limits.",
  },
  {
    n: "04",
    title: "Check and test",
    command: "tysel task verify",
    copy: "Type, capability, import, and isolate tests before you package.",
  },
  {
    n: "05",
    title: "Run, then ship one file",
    command: "tysel dev && tysel task release",
    copy: "Reload in development, then emit one executable plus release evidence.",
  },
];

export function FiveMinutes() {
  const [index, setIndex] = useState(0);
  const [paused, setPaused] = useState(false);
  const step = steps[index];

  useEffect(() => {
    if (paused) return;
    const id = window.setInterval(() => {
      setIndex((current) => (current + 1) % steps.length);
    }, 3800);
    return () => window.clearInterval(id);
  }, [paused]);

  return (
    <div
      className="grid gap-6 lg:grid-cols-[minmax(0,0.9fr)_minmax(0,1.1fr)]"
      onMouseEnter={() => setPaused(true)}
      onMouseLeave={() => setPaused(false)}
    >
      <ol className="flex flex-col">
        {steps.map((item, i) => (
          <li key={item.n}>
            <button
              type="button"
              onClick={() => setIndex(i)}
              className={`flex w-full items-start gap-3 px-3 py-3 text-left transition-colors ${
                i === index ? "bg-fd-accent" : "hover:bg-fd-accent/60"
              }`}
            >
              <span className="font-mono text-xs text-tysel-blue">{item.n}</span>
              <span>
                <span className="block text-sm font-medium">{item.title}</span>
                <span className="mt-1 block font-mono text-xs text-fd-muted-foreground">
                  $ {item.command}
                </span>
              </span>
            </button>
          </li>
        ))}
      </ol>
      <div className="home-terminal border border-fd-border bg-tysel-ink p-5 text-tysel-white">
        <p
          key={step.n}
          className="home-code-fade font-mono text-xs text-tysel-lime"
        >
          $ {step.command}
        </p>
        <p
          key={`${step.n}-copy`}
          className="home-code-fade mt-4 text-sm leading-6 text-white/70"
        >
          {step.copy}
        </p>
        <p className="mt-6 text-xs text-white/40">
          Node.js is optional for editor declarations. The packaged application
          does not need it. {paused ? "Paused." : "Hover to pause."}
        </p>
      </div>
    </div>
  );
}
