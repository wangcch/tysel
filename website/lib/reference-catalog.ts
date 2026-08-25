export type ReferenceTile = {
  name: string;
  description: string;
  href: string;
};

export type ReferenceSection = {
  title: string;
  blurb?: string;
  tiles: ReferenceTile[];
};

/**
 * Flat reference catalog for `/reference`. The sidebar keeps the full tree;
 * this is the scannable index (bun.sh-style tiles, Tysel groupings).
 */
export const referenceCatalog: ReferenceSection[] = [
  {
    title: "Runtime",
    blurb: "Handlers, host grants, and durable execution.",
    tiles: [
      {
        name: "Runtime",
        description: "Overview of exports and globalThis.tysel",
        href: "/reference/runtime",
      },
      {
        name: "Core types",
        description: "JsonValue, ExecutionProfile, TrustMode",
        href: "/reference/runtime/types",
      },
      {
        name: "Application module",
        description: "fetch, cron, queue, and MCP exports",
        href: "/reference/runtime/application",
      },
      {
        name: "Host capabilities",
        description: "secrets, SQL, filesystem, LLM, WebSocket",
        href: "/reference/runtime/capabilities",
      },
      {
        name: "Durable API",
        description: "effects, sleep, signals, resume",
        href: "/reference/runtime/durable",
      },
      {
        name: "Testing API",
        description: "test, assert, invokeFetch",
        href: "/reference/runtime/testing",
      },
    ],
  },
  {
    title: "Web APIs",
    blurb: "Bounded server-side globals — not a Node compatibility layer.",
    tiles: [
      {
        name: "JavaScript APIs",
        description: "Index and compatibility inventory",
        href: "/reference/javascript",
      },
      {
        name: "fetch",
        description: "Allowlisted outbound HTTP",
        href: "/reference/javascript/fetch",
      },
      {
        name: "Request · Response",
        description: "Web-standard message types",
        href: "/reference/javascript/request",
      },
      {
        name: "Headers",
        description: "Case-insensitive header map",
        href: "/reference/javascript/headers",
      },
      {
        name: "URL",
        description: "URL and URLSearchParams",
        href: "/reference/javascript/url",
      },
      {
        name: "WebSocket",
        description: "Client and server socket subset",
        href: "/reference/javascript/websocket",
      },
      {
        name: "Crypto",
        description: "Web Crypto subset",
        href: "/reference/javascript/crypto",
      },
      {
        name: "Timers",
        description: "setTimeout and setInterval",
        href: "/reference/javascript/timers",
      },
      {
        name: "Event",
        description: "Event and EventTarget",
        href: "/reference/javascript/event",
      },
      {
        name: "AbortController",
        description: "AbortSignal for fetch and tasks",
        href: "/reference/javascript/abortcontroller",
      },
      {
        name: "TextEncoder",
        description: "UTF-8 encode and decode",
        href: "/reference/javascript/textencoder",
      },
    ],
  },
  {
    title: "Wasm Components",
    blurb: "Experimental one-shot tasks through the Component Model.",
    tiles: [
      {
        name: "Wasm Components",
        description: "Profile, trust mode, and task boundary",
        href: "/reference/component",
      },
      {
        name: "Component ABI",
        description: "tysel:component/task world and JSON I/O",
        href: "/reference/component/abi",
      },
      {
        name: "Runtime and WASI",
        description: "Portable execution limits and restricted WASI",
        href: "/reference/component/runtime",
      },
      {
        name: "Component capabilities",
        description: "Filesystem WIT imports and grants",
        href: "/reference/component/capabilities",
      },
      {
        name: "Rust SDK",
        description: "Guest types and dispatcher",
        href: "/reference/component/rust-sdk",
      },
      {
        name: "Go SDK",
        description: "Generated bindings and dispatcher",
        href: "/reference/component/go-sdk",
      },
    ],
  },
  {
    title: "CLI",
    blurb: "Installed command surface — help output stays authoritative.",
    tiles: [
      {
        name: "CLI",
        description: "Global syntax and command map",
        href: "/reference/cli",
      },
      {
        name: "Project commands",
        description: "init, config, schema",
        href: "/reference/cli/project",
      },
      {
        name: "Develop and test",
        description: "check, compat, test, dev, run, inspect",
        href: "/reference/cli/development",
      },
      {
        name: "Tasks and protocols",
        description: "task, queue, mcp",
        href: "/reference/cli/tasks",
      },
      {
        name: "Build and image",
        description: "build, image",
        href: "/reference/cli/delivery",
      },
      {
        name: "Installation",
        description: "doctor, upgrade",
        href: "/reference/cli/installation",
      },
      {
        name: "Evidence",
        description: "bench, release",
        href: "/reference/cli/evidence",
      },
    ],
  },
  {
    title: "Manifest and host",
    blurb: "tysel.toml keys, environment, limits, and machine output.",
    tiles: [
      {
        name: "Manifest",
        description: "Field index and validation rules",
        href: "/reference/manifest",
      },
      {
        name: "Application and server",
        description: "app identity and inbound server",
        href: "/reference/manifest/app-server",
      },
      {
        name: "Permissions",
        description: "Network, secrets, SQL, filesystem grants",
        href: "/reference/manifest/permissions",
      },
      {
        name: "Application limits",
        description: "Body size, concurrency, timeouts",
        href: "/reference/manifest/limits",
      },
      {
        name: "Durable and observability",
        description: "Store, logs, and telemetry keys",
        href: "/reference/manifest/durable-observability",
      },
      {
        name: "Manifest tasks",
        description: "Project workflow definitions",
        href: "/reference/manifest/tasks",
      },
      {
        name: "Environment variables",
        description: "Installer, CLI, and host adapters",
        href: "/reference/environment",
      },
      {
        name: "Limits and defaults",
        description: "Hard bounds across runtime surfaces",
        href: "/reference/limits-and-defaults",
      },
      {
        name: "Errors and output",
        description: "Exit codes and JSON envelopes",
        href: "/reference/errors-and-output",
      },
    ],
  },
  {
    title: "Related guides",
    blurb: "Profile and dependency contracts live in docs.",
    tiles: [
      {
        name: "Capability matrix",
        description: "Grants by service, isolated, and component",
        href: "/docs/capabilities",
      },
      {
        name: "npm compatibility",
        description: "Package scan and unsupported surfaces",
        href: "/docs/compatibility",
      },
      {
        name: "Example gallery",
        description: "Runnable acceptance paths in the repo",
        href: "/docs/guides/examples",
      },
    ],
  },
];
