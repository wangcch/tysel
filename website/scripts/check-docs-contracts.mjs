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

if (!process.exitCode) {
  console.log(
    `Documentation contracts cover ${commands.length} CLI commands, ` +
      `${configCommands.length} config commands, ${exports.length} public types, ` +
      `and ${manifestFields.length} manifest fields.`,
  );
}
