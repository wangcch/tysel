import { existsSync, lstatSync, readFileSync, readdirSync } from "node:fs";
import path from "node:path";
import { fileURLToPath } from "node:url";

const scriptDirectory = path.dirname(fileURLToPath(import.meta.url));
const repositoryRoot = path.resolve(scriptDirectory, "../..");
const docsRoot = path.join(repositoryRoot, "docs");
const configPath = path.join(repositoryRoot, "mkdocs.yml");
const config = readFileSync(configPath, "utf8");
const errors = [];

function relativeToDocs(filePath) {
  return path.relative(docsRoot, filePath).split(path.sep).join("/");
}

function listFiles(directory) {
  return readdirSync(directory, { withFileTypes: true }).flatMap((entry) => {
    const entryPath = path.join(directory, entry.name);
    return entry.isDirectory() ? listFiles(entryPath) : [entryPath];
  });
}

function parseExcludedDocuments(source) {
  const lines = source.split(/\r?\n/);
  const excluded = new Set();
  const start = lines.findIndex((line) => /^exclude_docs:\s*\|\s*$/.test(line));
  if (start === -1) return excluded;

  for (const line of lines.slice(start + 1)) {
    if (line.length > 0 && !/^\s/.test(line)) break;
    const value = line.trim();
    if (value && !value.startsWith("#")) excluded.add(value);
  }
  return excluded;
}

function parseNavigationDocuments(source) {
  const documents = [];
  for (const line of source.split(/\r?\n/)) {
    const match = line.match(/^\s*-\s+[^:]+:\s+([^#]+\.md)\s*$/);
    if (match) documents.push(match[1].trim());
  }
  return documents;
}

function analyzeMarkdown(source) {
  const visibleLines = [];
  let fence = null;

  for (const [index, line] of source.split(/\r?\n/).entries()) {
    const marker = line.match(/^\s*(`{3,}|~{3,})/);
    if (marker) {
      const candidate = marker[1];
      if (!fence) {
        fence = { character: candidate[0], length: candidate.length, line: index + 1 };
      } else if (
        candidate[0] === fence.character &&
        candidate.length >= fence.length
      ) {
        fence = null;
      }
      visibleLines.push("");
      continue;
    }
    visibleLines.push(fence ? "" : line);
  }

  return { visible: visibleLines.join("\n"), unclosedFence: fence };
}

function headingSlug(heading) {
  const custom = heading.match(/\s+\{#([^}]+)\}\s*$/);
  if (custom) return custom[1];

  return heading
    .replace(/\[([^\]]+)]\([^)]+\)/g, "$1")
    .replace(/<[^>]+>/g, "")
    .replace(/[`*_~]/g, "")
    .trim()
    .toLowerCase()
    .replace(/[^\p{L}\p{N}\s_-]/gu, "")
    .replace(/\s+/g, "-");
}

function headingIds(source) {
  const ids = new Set();
  const { visible } = analyzeMarkdown(source);
  for (const line of visible.split(/\r?\n/)) {
    const match = line.match(/^#{1,6}\s+(.+?)\s*#*\s*$/);
    if (match) ids.add(headingSlug(match[1]));
  }
  return ids;
}

function resolveDocumentTarget(filePath, target) {
  const candidate = path.resolve(path.dirname(filePath), target);
  if (!existsSync(candidate)) return { candidate };
  if (!lstatSync(candidate).isDirectory()) return { candidate, file: candidate };

  for (const indexName of ["index.md", "README.md"]) {
    const indexPath = path.join(candidate, indexName);
    if (existsSync(indexPath)) return { candidate, file: indexPath };
  }
  return { candidate };
}

const excludedDocuments = parseExcludedDocuments(config);
const navigationDocuments = parseNavigationDocuments(config);
const allMarkdown = listFiles(docsRoot)
  .filter((filePath) => filePath.endsWith(".md"))
  .sort();
const publicMarkdown = allMarkdown.filter(
  (filePath) => !excludedDocuments.has(relativeToDocs(filePath)),
);
const publicSet = new Set(publicMarkdown.map(relativeToDocs));
const navigationSet = new Set(navigationDocuments);

for (const navPath of navigationDocuments) {
  if (!existsSync(path.join(docsRoot, navPath))) {
    errors.push(`mkdocs.yml: navigation target does not exist: ${navPath}`);
  }
}

for (const navPath of navigationSet) {
  if (excludedDocuments.has(navPath)) {
    errors.push(`mkdocs.yml: excluded document appears in navigation: ${navPath}`);
  }
}

for (const documentPath of publicSet) {
  if (!navigationSet.has(documentPath)) {
    errors.push(`mkdocs.yml: public document is missing from navigation: ${documentPath}`);
  }
}

let localLinkCount = 0;

for (const filePath of publicMarkdown) {
  const relativePath = relativeToDocs(filePath);
  const source = readFileSync(filePath, "utf8");
  const { visible, unclosedFence } = analyzeMarkdown(source);
  if (unclosedFence) {
    errors.push(`${relativePath}:${unclosedFence.line}: unclosed code fence`);
  }

  const h1Count = visible.split(/\r?\n/).filter((line) => /^#\s+\S/.test(line)).length;
  if (h1Count !== 1) {
    errors.push(`${relativePath}: expected exactly one H1, found ${h1Count}`);
  }

  const seenHeadingIds = new Set();
  for (const [lineIndex, line] of visible.split(/\r?\n/).entries()) {
    const heading = line.match(/^#{1,6}\s+(.+?)\s*#*\s*$/);
    if (!heading) continue;
    const id = headingSlug(heading[1]);
    if (seenHeadingIds.has(id)) {
      errors.push(`${relativePath}:${lineIndex + 1}: duplicate heading anchor #${id}`);
    }
    seenHeadingIds.add(id);
  }

  const linkPattern = /!?\[[^\]]*\]\(([^)]+)\)/g;
  for (const match of visible.matchAll(linkPattern)) {
    const rawTarget = match[1].trim().replace(/\s+["'][^"']*["']$/, "");
    if (/^(?:https?:|mailto:|tel:|data:)/i.test(rawTarget)) continue;
    if (rawTarget.startsWith("/")) continue;

    const [rawPath, rawFragment] = rawTarget.split("#", 2);
    const decodedPath = decodeURIComponent(rawPath.split("?", 1)[0]);
    const decodedFragment = rawFragment ? decodeURIComponent(rawFragment) : "";
    const targetPath = decodedPath || relativePath;
    const resolved = decodedPath
      ? resolveDocumentTarget(filePath, decodedPath)
      : { candidate: filePath, file: filePath };
    localLinkCount += 1;

    const relativeCandidate = path.relative(docsRoot, resolved.candidate);
    if (relativeCandidate.startsWith(`..${path.sep}`) || relativeCandidate === "..") {
      errors.push(
        `${relativePath}: repository-relative link escapes docs and will not publish: ${rawTarget}`,
      );
      continue;
    }

    if (!resolved.file) {
      errors.push(`${relativePath}: local link target does not exist: ${targetPath}`);
      continue;
    }

    const resolvedRelative = relativeToDocs(resolved.file);
    if (excludedDocuments.has(resolvedRelative)) {
      errors.push(`${relativePath}: public page links to excluded document: ${rawTarget}`);
      continue;
    }

    if (decodedFragment && resolved.file.endsWith(".md")) {
      const targetSource = readFileSync(resolved.file, "utf8");
      if (!headingIds(targetSource).has(decodedFragment)) {
        errors.push(`${relativePath}: heading target does not exist: ${rawTarget}`);
      }
    }
  }
}

for (const machineIndex of ["llms.txt", "llms-small.txt"]) {
  if (!existsSync(path.join(docsRoot, machineIndex))) {
    errors.push(`docs/${machineIndex}: machine-readable documentation index is missing`);
  }
}

if (errors.length > 0) {
  console.error(`Documentation check failed with ${errors.length} error(s):`);
  for (const error of errors) console.error(`- ${error}`);
  process.exit(1);
}

console.log(
  `Documentation check passed: ${publicMarkdown.length} public Markdown pages, ` +
    `${navigationDocuments.length} navigation entries, ${localLinkCount} local links.`,
);
