import fs from "node:fs";
import path from "node:path";

const repoRoot = path.resolve(import.meta.dirname, "../..");
const docsRoot = path.join(repoRoot, "docs");
const websiteRoot = path.resolve(import.meta.dirname, "..");
const docsOut = path.join(websiteRoot, "content/docs");
const referenceOut = path.join(websiteRoot, "content/reference");

const skip = new Set(["website-plan.md", "documentation-roadmap.md"]);

const titles = {
  "index.md": "Tysel documentation",
  "install.md": "Install Tysel",
  "getting-started.md": "Getting started",
  "guides/index.md": "Guides",
  "guides/examples.md": "Example gallery",
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
  "architecture/README.md": "Architecture",
  "adr/index.md": "Architecture decision records",
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
    if (resolved.startsWith("..") || skip.has(path.basename(resolved))) return match;

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
  const para = [];
  for (const line of lines) {
    if (line.startsWith("#")) {
      started = true;
      continue;
    }
    if (!started) continue;
    if (line.trim() === "") {
      if (para.length) break;
      continue;
    }
    if (line.startsWith("```") || line.startsWith("|") || line.startsWith(">")) break;
    para.push(line.trim());
    if (para.join(" ").length > 180) break;
  }
  return para.join(" ").replaceAll('"', "'").slice(0, 220);
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
  fs.writeFileSync(
    outFile,
    `---\ntitle: ${JSON.stringify(title)}\ndescription: ${JSON.stringify(description)}\n---\n\n${body}`,
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
[architecture notes](/docs/architecture/javascript-runtime-compatibility).
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

Rationale and evidence live in
[JavaScript runtime compatibility](/docs/architecture/javascript-runtime-compatibility).
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
  if (skip.has(path.basename(rel))) continue;

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
      "---Internals---",
      "architecture",
      "adr",
    ],
  },
  "guides/meta.json": { title: "Guides", pages: ["index", "examples"] },
  "concepts/meta.json": {
    title: "Learn",
    pages: [
      "how-tysel-works",
      "projects-and-configuration",
      "execution-profiles",
      "durable-execution",
    ],
  },
  "architecture/meta.json": {
    title: "Architecture",
    pages: ["index", "javascript-runtime-compatibility", "javascript-runtime-convergence"],
  },
  "adr/meta.json": {
    title: "Decisions",
    pages: [
      "index",
      "001-runtime-core-rust",
      "002-quickjs-ng-engine",
      "003-web-api-first",
      "004-build-once-ship-one-file",
      "005-deny-by-default",
      "006-process-isolation",
      "007-durable-replay",
      "008-wit-capability-abi",
      "009-no-aot-on-v1-path",
      "010-static-typescript-parallel",
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

const imported = walk(docsRoot).filter((file) => !skip.has(path.basename(file))).length;
console.log(
  `Imported ${imported} markdown pages and generated ${jsCount} JavaScript API pages.`,
);
