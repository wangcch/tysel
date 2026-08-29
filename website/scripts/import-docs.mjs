import fs from "node:fs";
import path from "node:path";

const repoRoot = path.resolve(import.meta.dirname, "../..");
const docsRoot = path.join(repoRoot, "docs");
const websiteRoot = path.resolve(import.meta.dirname, "..");
const docsOut = path.join(websiteRoot, "content/docs");
const referenceOut = path.join(websiteRoot, "content/reference");

const publicDocs = {
  files: new Set(["index.md", "install.md", "getting-started.md"]),
  directories: new Set([
    "guides",
    "concepts",
    "capabilities",
    "compatibility",
    "security",
    "operations",
    "performance",
    "reference",
  ]),
};

const titles = {
  "index.md": "Tysel documentation",
  "install.md": "Install Tysel",
  "getting-started.md": "Getting started",
  "guides/index.md": "Guides",
  "guides/examples.md": "Example gallery",
  "guides/service-networking.md": "Service networking",
  "guides/concurrency-backpressure.md": "Concurrency and backpressure",
  "guides/cron-queue.md": "Cron and Queue",
  "guides/llm-gateway.md": "LLM gateway",
  "guides/filesystem.md": "Filesystem",
  "guides/sqlite.md": "SQLite",
  "guides/postgresql.md": "PostgreSQL",
  "guides/redis.md": "Redis",
  "guides/container-image.md": "Container image",
  "guides/observability.md": "Observability",
  "guides/debugging.md": "Debugging",
  "guides/reproducible-release.md": "Reproducible release",
  "concepts/how-tysel-works.md": "How Tysel works",
  "concepts/projects-and-configuration.md": "Projects and configuration",
  "concepts/execution-profiles.md": "Execution profiles",
  "concepts/durable-execution.md": "Durable execution",
  "reference/index.md": "Overview",
  "capabilities/README.md": "Capability matrix",
  "compatibility/README.md": "npm compatibility",
  "security/README.md": "Security model",
  "operations/production.md": "Production operations",
  "performance/README.md": "Performance and evidence",
  "performance/redis.md": "Redis performance evaluation",
};

function walk(dir) {
  const out = [];
  for (const entry of fs.readdirSync(dir, { withFileTypes: true })) {
    const full = path.join(dir, entry.name);
    if (entry.isDirectory()) out.push(...walk(full));
    else if (entry.name.endsWith(".md")) out.push(full);
  }
  return out;
}

function posixRel(rel) {
  return rel.split(path.sep).join("/");
}

function shouldPublish(rel) {
  const normalized = posixRel(rel);
  const [directory] = normalized.split("/");
  return publicDocs.files.has(normalized) || publicDocs.directories.has(directory);
}

function isReferenceRel(rel) {
  const normalized = posixRel(rel);
  return normalized === "reference" || normalized.startsWith("reference/");
}

function toOutFile(rel) {
  const renamed = posixRel(rel)
    .replace(/README\.md$/, "index.mdx")
    .replace(/\.md$/, ".mdx");
  if (isReferenceRel(rel)) return path.join(websiteRoot, "content", renamed);
  return path.join(docsOut, renamed);
}

function publicUrlFromRel(rel) {
  const trimmed = posixRel(rel)
    .replace(/README\.md$/, "")
    .replace(/index\.md$/, "")
    .replace(/\.md$/, "")
    .replace(/\/$/, "");

  if (trimmed === "reference" || trimmed.startsWith("reference/")) {
    const rest = trimmed === "reference" ? "" : trimmed.slice("reference/".length);
    return rest ? `/reference/${rest}` : "/reference";
  }

  return trimmed ? `/docs/${trimmed}` : "/docs";
}

function rewriteLinks(content, fromRel) {
  return content.replace(/\]\(([^)]+)\)/g, (match, href) => {
    if (
      href.startsWith("http://") ||
      href.startsWith("https://") ||
      href.startsWith("mailto:") ||
      href.startsWith("#") ||
      href.startsWith("/docs/") ||
      href.startsWith("/reference/")
    ) {
      return match;
    }

    const [file, hash] = href.split("#");
    if (!file || (!file.endsWith(".md") && !file.includes(".md") && !file.endsWith("README.md"))) {
      if (!file.endsWith(".md")) return match;
    }

    const resolved = path.normalize(path.join(path.dirname(fromRel), file));
    if (resolved.startsWith("..")) return match;
    if (!shouldPublish(resolved)) {
      throw new Error(
        `Public document ${posixRel(fromRel)} links non-public document ${posixRel(resolved)}`,
      );
    }

    return `](${publicUrlFromRel(resolved)}${hash ? `#${hash}` : ""})`;
  });
}

function extractTitle(content, rel) {
  if (titles[posixRel(rel)]) return titles[posixRel(rel)];
  const heading = content.match(/^#\s+(.+)$/m);
  if (heading) return heading[1].trim();
  return path.basename(rel, ".md");
}

function extractDescription(content) {
  const lines = content.split("\n");
  let started = false;
  let inFence = false;
  const para = [];
  const candidates = [];

  function flush() {
    if (!para.length) return;
    candidates.push(para.splice(0).join(" "));
  }

  for (const line of lines) {
    const trimmed = line.trim();
    if (line.startsWith("#")) {
      flush();
      started = true;
      continue;
    }
    if (!started) continue;
    if (trimmed.startsWith("```") || trimmed.startsWith("~~~")) {
      flush();
      inFence = !inFence;
      continue;
    }
    if (inFence) continue;
    if (trimmed === "") {
      flush();
      continue;
    }
    if (
      trimmed.startsWith("|") ||
      trimmed.startsWith(">") ||
      /^(?:[-+*]|\d+\.)\s+/.test(trimmed)
    ) {
      flush();
      continue;
    }
    para.push(trimmed);
  }
  flush();

  const description = candidates
    .map((candidate) =>
      candidate
        .replace(/\[([^\]]+)\]\([^)]+\)/g, "$1")
        .replace(/`([^`]+)`/g, "$1")
        .replace(/\*\*([^*]+)\*\*/g, "$1")
        .replace(/__([^_]+)__/g, "$1")
        .replace(/~~([^~]+)~~/g, "$1")
        .replace(/\*([^*]+)\*/g, "$1")
        .replaceAll('"', "'")
        .replace(/^(?:status|状态)\s*[:：][^.。]+[.。]\s*/i, "")
        .replace(/[:：]$/, ".")
        .trim(),
    )
    .find((candidate) => candidate.length >= 40);

  const plain = (description ?? "")
    .replace(/\s+/g, " ")
    .trim();

  if (plain.length <= 180) return plain;
  if (plain.length <= 220 && /[.!?]$/.test(plain)) return plain;

  const preview = plain.slice(0, 221);
  const sentenceEnds = [
    preview.lastIndexOf(". "),
    preview.lastIndexOf("? "),
    preview.lastIndexOf("! "),
  ];
  const sentenceEnd = Math.max(...sentenceEnds);
  if (sentenceEnd >= 40) return plain.slice(0, sentenceEnd + 1).trim();

  const cutoff = plain.slice(0, 181).lastIndexOf(" ");
  return `${plain.slice(0, cutoff).replace(/[,:;]$/, "").trim()}…`;
}

function escapeMdx(content) {
  const chunks = content.split(/(```[\s\S]*?```)/g);
  return chunks
    .map((chunk, index) => {
      if (index % 2 === 1) return chunk;
      return chunk
        .split("\n")
        .map((line) => {
          if (line.trimStart().startsWith("|")) {
            return line.replace(/`([^`\n]*<[^`\n]*)`/g, (_, code) => {
              const escaped = code
                .replaceAll("\\", "\\\\")
                .replaceAll('"', '\\"')
                .replaceAll("|", "\\|");
              return `<code>{"${escaped}"}</code>`;
            });
          }
          return line
            .split(/(`[^`\n]+`)/g)
            .map((segment, segmentIndex) => {
              if (segmentIndex % 2 === 1) return segment;
              return segment.replace(/<(?=[A-Za-z_/])/g, "\\<");
            })
            .join("");
        })
        .join("\n");
    })
    .join("");
}

function writeMdx(outFile, title, description, body) {
  fs.mkdirSync(path.dirname(outFile), { recursive: true });
  const bodyWithoutTitle = body.replace(/^\s*#\s+.*\r?\n(?:\r?\n)?/, "");
  fs.writeFileSync(
    outFile,
    `---\ntitle: ${JSON.stringify(title)}\ndescription: ${JSON.stringify(description)}\n---\n\n${bodyWithoutTitle}`,
  );
}

function apiSlug(name) {
  return name
    .split(" and ")[0]
    .trim()
    .toLowerCase()
    .replace(/[^a-z0-9]+/g, "-")
    .replace(/^-|-$/g, "");
}

function generateJavascriptReference() {
  const inventoryPath = path.join(repoRoot, "runtime-js/web-api/compatibility.json");
  const inventory = JSON.parse(fs.readFileSync(inventoryPath, "utf8"));
  const dir = path.join(referenceOut, "javascript");
  fs.mkdirSync(dir, { recursive: true });

  const entries = inventory.apis.map((api) => ({ ...api, slug: apiSlug(api.name) }));

  for (const api of entries) {
    const supported = api.supported.map((item) => `- ${item}`).join("\n");
    const unsupported = api.unsupported.length
      ? api.unsupported.map((item) => `- ${item}`).join("\n")
      : "- None listed in the compatibility inventory.";

    writeMdx(
      path.join(dir, `${api.slug}.mdx`),
      api.name,
      `Supported ${api.name} contract in the Tysel JavaScript runtime.`,
      `# ${api.name}

Status: \`${api.status}\`. This page is the supported server-side subset, not
the full browser specification. Behavior outside the lists below is not a
contract.

Source of truth: \`runtime-js/web-api/compatibility.json\`
(\`${inventory.profile}\`, ${inventory.stability}).

## Supported

${supported}

## Not supported

${unsupported}

See the [JavaScript API index](/reference/javascript) and the
[npm compatibility guide](/docs/compatibility).
`,
    );
  }

  const table = entries
    .map((api) => `| [${api.name}](/reference/javascript/${api.slug}) | \`${api.status}\` |`)
    .join("\n");

  writeMdx(
    path.join(dir, "index.mdx"),
    "JavaScript APIs",
    "The supported server-side Web API subset. Each page is the contract for one global.",
    `# JavaScript APIs

Tysel implements a bounded server-side Web API profile. Use these pages to look
up a global. Use [guides](/docs/guides) when you want a workflow.

\`partial\` means the listed behavior is a supported contract, not that the
corresponding browser specification is implemented in full. The machine-readable
inventory is \`runtime-js/web-api/compatibility.json\`.

| API | Status |
| --- | --- |
${table}
`,
  );

  fs.writeFileSync(
    path.join(dir, "meta.json"),
    `${JSON.stringify({ title: "JavaScript", pages: ["index", ...entries.map((api) => api.slug)] }, null, 2)}\n`,
  );

  return entries.length;
}

fs.rmSync(docsOut, { recursive: true, force: true });
fs.rmSync(referenceOut, { recursive: true, force: true });
fs.mkdirSync(docsOut, { recursive: true });
fs.mkdirSync(referenceOut, { recursive: true });

for (const file of walk(docsRoot)) {
  const rel = path.relative(docsRoot, file);
  if (!shouldPublish(rel)) continue;

  let body = fs.readFileSync(file, "utf8");
  if (body.startsWith("---\n")) {
    body = body.replace(/^---\n[\s\S]*?\n---\n/, "");
  }

  writeMdx(
    toOutFile(rel),
    extractTitle(body, rel),
    extractDescription(body),
    escapeMdx(rewriteLinks(body, rel)),
  );
}

const jsCount = generateJavascriptReference();

const docsMetas = {
  "meta.json": {
    title: "Tysel",
    pages: [
      "index",
      "---Start---",
      "install",
      "getting-started",
      "---Guides---",
      "guides",
      "---Learn---",
      "concepts",
      "---Lookup---",
      "capabilities",
      "compatibility",
      "---Operate---",
      "security",
      "operations",
      "performance",
    ],
  },
  "guides/meta.json": {
    title: "Guides",
    pages: [
      "index",
      "examples",
      "service-networking",
      "concurrency-backpressure",
      "cron-queue",
      "llm-gateway",
      "filesystem",
      "sqlite",
      "postgresql",
      "redis",
      "container-image",
      "observability",
      "debugging",
      "reproducible-release",
      "wasm-component-rust",
      "wasm-component-go",
    ],
  },
  "concepts/meta.json": {
    title: "Learn",
    pages: [
      "how-tysel-works",
      "projects-and-configuration",
      "execution-profiles",
      "durable-execution",
    ],
  },
  "operations/meta.json": { title: "Operate", pages: ["production"] },
};

const referenceMetas = {
  "meta.json": {
    title: "Reference",
    pages: [
      "index",
      "javascript",
      "runtime",
      "component",
      "cli",
      "manifest",
      "environment",
      "limits-and-defaults",
      "errors-and-output",
      "[Capability matrix](/docs/capabilities)",
      "[npm compatibility](/docs/compatibility)",
    ],
  },
  "cli/meta.json": {
    title: "CLI",
    pages: ["index", "project", "development", "tasks", "delivery", "installation", "evidence"],
  },
  "manifest/meta.json": {
    title: "Manifest",
    pages: ["index", "app-server", "permissions", "limits", "durable-observability", "tasks"],
  },
  "runtime/meta.json": {
    title: "Runtime",
    pages: ["index", "types", "application", "capabilities", "durable", "testing"],
  },
};

for (const [rel, json] of Object.entries(docsMetas)) {
  const file = path.join(docsOut, rel);
  fs.mkdirSync(path.dirname(file), { recursive: true });
  fs.writeFileSync(file, JSON.stringify(json, null, 2) + "\n");
}

for (const [rel, json] of Object.entries(referenceMetas)) {
  const file = path.join(referenceOut, rel);
  fs.mkdirSync(path.dirname(file), { recursive: true });
  fs.writeFileSync(file, JSON.stringify(json, null, 2) + "\n");
}

const imported = walk(docsRoot).filter(
  (file) => shouldPublish(path.relative(docsRoot, file)),
).length;
console.log(
  `Imported ${imported} markdown pages and generated ${jsCount} JavaScript API pages.`,
);
