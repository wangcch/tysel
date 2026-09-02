import fs from "node:fs";
import path from "node:path";

const repoRoot = path.resolve(import.meta.dirname, "../..");

function read(relative) {
  return fs.readFileSync(path.join(repoRoot, relative), "utf8");
}

function fail(group, missing) {
  if (missing.length === 0) return;
  console.error(`${group} missing from documentation:`);
  for (const item of missing) console.error(`  - ${item}`);
  process.exitCode = 1;
}

function markdownFiles(directory) {
  const files = [];
  for (const entry of fs.readdirSync(directory, { withFileTypes: true })) {
    const full = path.join(directory, entry.name);
    if (entry.isDirectory()) files.push(...markdownFiles(full));
    else if (entry.name.endsWith(".md")) files.push(full);
  }
  return files;
}

const unbalancedCodeSpans = [];
for (const file of markdownFiles(path.join(repoRoot, "docs"))) {
  let inFence = false;
  for (const [index, line] of fs.readFileSync(file, "utf8").split("\n").entries()) {
    if (/^\s*```/.test(line)) {
      inFence = !inFence;
      continue;
    }
    if (inFence) continue;
    const delimiters = line.match(/(?<!\\)`+/g) ?? [];
    if (delimiters.length % 2 !== 0) {
      unbalancedCodeSpans.push(`${path.relative(repoRoot, file)}:${index + 1}`);
    }
  }
}
fail("Balanced Markdown code spans", unbalancedCodeSpans);

function enumVariants(source, enumName) {
  const start = source.indexOf(`enum ${enumName} {`);
  if (start < 0) throw new Error(`cannot find Rust enum ${enumName}`);
  const lines = source.slice(start).split("\n").slice(1);
  const variants = [];
  for (const line of lines) {
    if (line === "}") break;
    const match = line.match(/^    ([A-Z][A-Za-z0-9_]*)(?:\s*\{|,)/);
    if (match) variants.push(match[1]);
  }
  return variants;
}

function kebab(value) {
  return value
    .replace(/([a-z0-9])([A-Z])/g, "$1-$2")
    .replaceAll("_", "-")
    .toLowerCase();
}

const cliSource = read("crates/tysel-cli/src/main.rs");
const cliDocs = fs
  .readdirSync(path.join(repoRoot, "docs/reference/cli"))
  .filter((name) => name.endsWith(".md"))
  .map((name) => read(`docs/reference/cli/${name}`))
  .join("\n");

const commands = enumVariants(cliSource, "Commands").map(kebab);
fail("CLI commands", commands.filter((command) => !cliDocs.includes(`tysel ${command}`)));

const configCommands = enumVariants(cliSource, "ConfigCommand").map(kebab);
fail(
  "CLI config commands",
  configCommands.filter((command) => !cliDocs.includes(`tysel config ${command}`)),
);

const typeSource = read("packages/tysel-types/src/index.ts");
const runtimeDocs = fs
  .readdirSync(path.join(repoRoot, "docs/reference/runtime"))
  .filter((name) => name.endsWith(".md"))
  .map((name) => read(`docs/reference/runtime/${name}`))
  .join("\n");
const exports = [
  ...typeSource.matchAll(/^export\s+(?:type|interface)\s+([A-Za-z0-9_]+)/gm),
].map((match) => match[1]);
fail(
  "@tysel/types exports",
  exports.filter((name) => !new RegExp(`\\b${name}\\b`).test(runtimeDocs)),
);

const schema = JSON.parse(read("crates/tysel-manifest/schema/tysel-manifest-v1.schema.json"));
const manifestDocs = read("docs/reference/manifest/index.md");
const manifestFields = [];
for (const [root, definition] of Object.entries(schema.properties)) {
  if (root === "tasks") {
    for (const field of Object.keys(schema.$defs.task.properties)) {
      manifestFields.push(`tasks.<name>.${field}`);
    }
  } else if (definition.properties) {
    for (const field of Object.keys(definition.properties)) {
      manifestFields.push(`${root}.${field}`);
    }
  } else {
    manifestFields.push(root);
  }
}
fail(
  "Manifest schema fields",
  manifestFields.filter((field) => !manifestDocs.includes(`\`${field}\``)),
);

const manifestValueFailures = [];
for (const [root, definition] of Object.entries(schema.properties)) {
  if (!definition.properties) continue;
  for (const [name, field] of Object.entries(definition.properties)) {
    const fieldName = `${root}.${name}`;
    const row = manifestDocs
      .split("\n")
      .find((line) => line.startsWith(`| \`${fieldName}\` |`));
    if (!row) continue;
    const expectedValues = [];
    if (Object.hasOwn(field, "default")) expectedValues.push(field.default);
    if (Object.hasOwn(field, "const")) expectedValues.push(field.const);
    if (Array.isArray(field.enum)) expectedValues.push(...field.enum);
    for (const value of expectedValues) {
      if (!row.includes(`\`${String(value)}\``)) {
        manifestValueFailures.push(`${fieldName} value ${JSON.stringify(value)}`);
      }
    }
  }
}
fail("Manifest defaults and enums", manifestValueFailures);

const developmentManifest = read("examples/hello-service/tysel.toml");
const containerManifest = read("examples/hello-service/tysel.container.toml");
const expectedContainerManifest = developmentManifest.replace(
  'listen = "127.0.0.1:3000"',
  'listen = "0.0.0.0:3000"',
);
fail(
  "Hello-service container manifest",
  containerManifest === expectedContainerManifest
    ? []
    : ["must equal tysel.toml except for the container listener"],
);

const containerGuide = read("docs/guides/container-image.md");
const imageReference = read("docs/reference/cli/delivery.md");
const imageOptions = ["--builder", "--copy-sidecars", "--image-version", "--label"];
fail(
  "Image CLI options",
  imageOptions.filter((option) => !imageReference.includes(`\`${option}`)),
);
const componentTasks = read("docs/operations/component-tasks.md");
fail(
  "Component image boundary",
  componentTasks.includes("`tysel image` rejects") &&
    read("crates/tysel-cli/src/image.rs").includes("docs/operations/component-tasks.md")
    ? []
    : ["component image rejection and its deployment page must remain linked"],
);
const continuousDelivery = read("docs/operations/continuous-delivery.md");
fail(
  "Continuous delivery identities",
  [
    "--copy-sidecars",
    "ELF architecture",
    "io.tysel.artifact.digest",
    "org.opencontainers.image.version",
    "Image or OCI index digest",
  ].filter((value) => !continuousDelivery.includes(value)),
);
const toolchainImage = "ghcr.io/wangcch/tysel-toolchain";
const toolchainDockerfile = read(".github/docker/toolchain.Dockerfile");
const releaseWorkflow = read(".github/workflows/release.yml");
const checkedInDockerfile = read("examples/hello-service/Dockerfile").trim();
const documentedDockerfile = containerGuide.match(
  /## Build from source in Docker[\s\S]*?```dockerfile\n([\s\S]*?)\n```/,
)?.[1];
fail(
  "Container Dockerfile",
  documentedDockerfile === checkedInDockerfile
    ? []
    : ["guide block must match examples/hello-service/Dockerfile"],
);
fail(
  "Toolchain image reference",
  [
    [containerGuide, "container guide"],
    [checkedInDockerfile, "hello-service Dockerfile"],
    [releaseWorkflow, "release workflow"],
  ]
    .filter(([source]) => !source.includes(toolchainImage))
    .map(([, name]) => `${name} must use ${toolchainImage}`),
);
fail(
  "Reproducible toolchain image",
  releaseWorkflow.includes('--build-arg "SOURCE_DATE_EPOCH=${source_date_epoch}"')
    ? []
    : ["release workflow must set the toolchain image timestamp from the source commit"],
);
fail(
  "Toolchain base image",
  /^ARG TOOLCHAIN_BASE_IMAGE=[^\s]+@sha256:[0-9a-f]{64}$/m.test(toolchainDockerfile)
    ? []
    : [".github/docker/toolchain.Dockerfile must pin a multi-platform base digest"],
);

const checkedInRuntimeDockerfile = read("examples/hello-service/Dockerfile.runtime").trim();
const documentedRuntimeDockerfile = containerGuide.match(
  /## Package an existing executable[\s\S]*?```dockerfile\n([\s\S]*?)\n```/,
)?.[1];
fail(
  "Runtime-only Dockerfile",
  documentedRuntimeDockerfile === checkedInRuntimeDockerfile
    ? []
    : ["guide block must match examples/hello-service/Dockerfile.runtime"],
);

const checkedInDockerignore = read("examples/hello-service/.dockerignore").trim();
const documentedDockerignore = containerGuide.match(
  /checked-in `\.dockerignore` is:\n\n```text\n([\s\S]*?)\n```/,
)?.[1];
fail(
  "Container .dockerignore",
  documentedDockerignore === checkedInDockerignore
    ? []
    : ["guide block must match examples/hello-service/.dockerignore"],
);

if (!process.exitCode) {
  console.log(
    `Documentation contracts cover ${commands.length} CLI commands, ` +
      `${configCommands.length} config commands, ${exports.length} public types, ` +
      `${manifestFields.length} manifest fields, and the checked container examples.`,
  );
}
