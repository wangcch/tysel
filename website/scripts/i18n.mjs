import { navigationIdentity, localizeNavigation } from "./navigation.mjs";
import fs from "node:fs";
import path from "node:path";
import crypto from "node:crypto";
import { fileURLToPath } from "node:url";

export const root = path.resolve(path.dirname(fileURLToPath(import.meta.url)), "..");
export const hash = (text) => crypto.createHash("sha256").update(text).digest("hex");
const read = (file) => JSON.parse(fs.readFileSync(file, "utf8"));
export function loadConfig(base = root) {
  const config = read(path.join(base, "locales/config.json"));
  if (!config.locales?.[config.sourceLocale]?.published) throw new Error("Source locale must be published");
  for (const locale of Object.keys(config.locales)) {
    if (!/^[a-z]{2,3}(?:-[A-Za-z0-9]{2,8})*$/.test(locale)) throw new Error(`Invalid locale code: ${locale}`);
  }
  return config;
}
export function sourceUnits(base = root) {
  const config = loadConfig(base);
  const units = Object.entries(read(path.join(base, `locales/${config.sourceLocale}/messages.json`)))
    .map(([id, source]) => ({ kind: "message", id, source, sourceHash: hash(source) }));
  const walk = (dir) => fs.existsSync(dir) ? fs.readdirSync(dir, { withFileTypes: true }).flatMap((entry) => {
    const file = path.join(dir, entry.name);
    return entry.isDirectory() ? walk(file) : /\.(mdx|json)$/.test(file) ? [file] : [];
  }) : [];
  for (const collection of ["docs", "reference", "blog"]) {
    for (const file of walk(path.join(base, "content", collection))) {
      const source = fs.readFileSync(file, "utf8");
      const id = `${collection}/${path.relative(path.join(base, "content", collection), file).split(path.sep).join("/")}`;
      units.push({ kind: "content", id, source, sourceHash: hash(source) });
    }
  }
  return units;
}
export function localeState(locale, base = root) {
  const dir = path.join(base, "locales", locale);
  const messages = read(path.join(dir, "messages.json")), manifest = read(path.join(dir, "manifest.json"));
  if (manifest.version !== 1 || !manifest.messages || !manifest.content) throw new Error(`Invalid manifest: ${locale}`);
  return { messages, manifest };
}
export function exportJob(locale, base = root) {
  const config = loadConfig(base);
  if (!Object.hasOwn(config.locales, locale) || locale === config.sourceLocale) throw new Error("Expected a configured target locale");
  const state = localeState(locale, base);
  const glossaryFile = path.join(base, "locales", locale, "glossary.json");
  const glossary = fs.existsSync(glossaryFile) ? read(glossaryFile) : {};
  return { version: 1, sourceLocale: config.sourceLocale, targetLocale: locale, glossary,
    instructions: "Translate prose only. Preserve placeholders, code fences, URLs, frontmatter structure, API names, and metadata page identifiers. Return each unit with a translation field. This command never calls a model.",
    units: sourceUnits(base).filter((unit) => state.manifest[unit.kind === "message" ? "messages" : "content"][unit.id]?.sourceHash !== unit.sourceHash)
  };
}
const placeholders = (text) => [...text.matchAll(/\{([A-Za-z][\w.]*)\}/g)].map((m) => m[1]).sort();
const fences = (text) => [...text.matchAll(/^(`{3,}|~{3,})[^\n]*\n[\s\S]*?^\1\s*$/gm)].map((m) => m[0]);
const links = (text) => [...text.matchAll(/\]\(([^)]+)\)/g)].map((m) => m[1]).sort();
export function validateTranslation(unit, translation) {
  if (typeof translation !== "string" || !translation.trim()) throw new Error(`Empty translation: ${unit.id}`);
  if (unit.kind === "message") {
    if (JSON.stringify(placeholders(unit.source)) !== JSON.stringify(placeholders(translation))) throw new Error(`Placeholder mismatch: ${unit.id}`);
  } else if (unit.id.endsWith(".mdx")) {
    if (JSON.stringify(fences(unit.source)) !== JSON.stringify(fences(translation))) throw new Error(`Code blocks changed: ${unit.id}`);
    if (JSON.stringify(links(unit.source)) !== JSON.stringify(links(translation))) throw new Error(`Link targets changed: ${unit.id}`);
    const metadata = (text) => {
      const match = text.match(/^---\r?\n([\s\S]*?)\r?\n---(?:\r?\n|$)/);
      if (!match) throw new Error(`Missing frontmatter: ${unit.id}`);
      return match[1].split(/\r?\n(?=[A-Za-z_][\w-]*:)/).filter((block) => !/^(title|description|coverAlt):/.test(block)).join("\n");
    };
    if (metadata(unit.source) !== metadata(translation)) throw new Error(`Immutable frontmatter changed: ${unit.id}`);
  } else {
    const source = JSON.parse(unit.source), target = JSON.parse(translation);
    // Page ordering/IDs and navigation behavior are never translation targets.
    for (const key of new Set([...Object.keys(source), ...Object.keys(target)])) {
      if (!['title', 'description'].includes(key) && JSON.stringify(key === 'pages' ? source[key]?.map(navigationIdentity) : source[key]) !== JSON.stringify(key === 'pages' ? target[key]?.map(navigationIdentity) : target[key])) throw new Error(`Navigation metadata changed: ${unit.id}:${key}`);
    }
  }
}
export function importJob(job, { base = root, reviewed = false } = {}) {
  const config = loadConfig(base);
  if (job.version !== 1 || job.sourceLocale !== config.sourceLocale || !Object.hasOwn(config.locales, job.targetLocale) || job.targetLocale === config.sourceLocale || !Array.isArray(job.units)) throw new Error("Invalid translation job");
  const current = new Map(sourceUnits(base).map((unit) => [`${unit.kind}:${unit.id}`, unit]));
  const seen = new Set(), accepted = [];
  for (const unit of job.units) {
    const key = `${unit.kind}:${unit.id}`, source = current.get(key);
    if (seen.has(key)) throw new Error(`Duplicate unit: ${key}`);
    seen.add(key);
    if (!source || source.sourceHash !== unit.sourceHash) throw new Error(`Unknown or stale source: ${key}`);
    validateTranslation(source, unit.translation);
    accepted.push({ ...source, translation: unit.translation });
  }
  // Validate the whole batch before writing anything. IDs must match known source units.
  const state = localeState(job.targetLocale, base), dir = path.join(base, "locales", job.targetLocale);
  for (const unit of accepted) {
    const entry = { sourceHash: unit.sourceHash, translationHash: hash(unit.translation), status: reviewed ? "reviewed" : "draft" };
    if (unit.kind === "message") { state.messages[unit.id] = unit.translation; state.manifest.messages[unit.id] = entry; }
    else {
      const file = path.join(dir, "content", unit.id);
      fs.mkdirSync(path.dirname(file), { recursive: true });
      fs.writeFileSync(file, unit.translation);
      state.manifest.content[unit.id] = entry;
    }
  }
  fs.writeFileSync(path.join(dir, "messages.json"), JSON.stringify(state.messages, null, 2) + "\n");
  fs.writeFileSync(path.join(dir, "manifest.json"), JSON.stringify(state.manifest, null, 2) + "\n");
  return accepted.length;
}
export function check(base = root) {
  const config = loadConfig(base), units = sourceUnits(base), issues = [];
  for (const [locale, settings] of Object.entries(config.locales)) {
    if (locale === config.sourceLocale) continue;
    const state = localeState(locale, base);
    for (const unit of units) {
      const entry = state.manifest[unit.kind === "message" ? "messages" : "content"][unit.id];
      const required = settings.published && unit.kind === "message";
      if (!entry && !required) continue;
      if (!entry || entry.status !== "reviewed" || entry.sourceHash !== unit.sourceHash) {
        if (settings.published) issues.push(`${locale}:${unit.kind}:${unit.id} is missing, draft, or stale`);
        continue;
      }
      try {
        const translation = unit.kind === "message" ? state.messages[unit.id] : fs.readFileSync(path.join(base, "locales", locale, "content", unit.id), "utf8");
        validateTranslation(unit, translation);
        if (entry.translationHash !== hash(translation)) throw new Error(`Translation changed since review: ${unit.id}`);
      } catch (error) { issues.push(`${locale}: ${error.message}`); }
    }
    const knownMessages = new Set(units.filter((unit) => unit.kind === "message").map((unit) => unit.id));
    for (const id of Object.keys(state.messages)) if (!knownMessages.has(id)) issues.push(`${locale}:messages:${id} is unknown`);
    for (const kind of ["messages", "content"]) {
      const known = new Set(units.filter((u) => u.kind === (kind === "messages" ? "message" : "content")).map((u) => u.id));
      for (const id of Object.keys(state.manifest[kind])) if (!known.has(id)) issues.push(`${locale}:${kind}:${id} is orphaned`);
    }
  }
  return issues;
}
export function prepare(base = root) {
  const issues = check(base);
  if (issues.length) throw new Error(issues.join("\n"));
  const target = path.join(base, "content/translations");
  fs.rmSync(target, { recursive: true, force: true });
  for (const collection of ["docs", "reference", "blog"]) fs.mkdirSync(path.join(target, collection), { recursive: true });
  const config = loadConfig(base), units = sourceUnits(base);
  // Next static export rejects empty generateStaticParams. Emit route adapters
  // only when a target language is published; there are no placeholder URLs.
  const routeRoot = path.join(base, "app/(localized)");
  fs.rmSync(routeRoot, { recursive: true, force: true });
  if (Object.entries(config.locales).some(([locale, settings]) => locale !== config.sourceLocale && settings.published)) {
    const routes = {
      "[lang]/template.tsx": 'export { default } from "@/app/(site)/template";\n',
      "[lang]/layout.tsx": 'export { default, metadata } from "@/lib/i18n/routes/layout";\n',
      "[lang]/[[...path]]/page.tsx": 'export { default, generateMetadata, generateStaticParams } from "@/lib/i18n/routes/page";\nexport const dynamic = "force-static";\nexport const dynamicParams = false;\n',
      "[lang]/search.json/route.ts": 'export { GET, generateStaticParams } from "@/lib/i18n/routes/search";\nexport const dynamic = "force-static";\nexport const dynamicParams = false;\n',
    };
    for (const [name, source] of Object.entries(routes)) {
      const file = path.join(routeRoot, name);
      fs.mkdirSync(path.dirname(file), { recursive: true }); fs.writeFileSync(file, source);
    }
  }
  for (const [locale, settings] of Object.entries(config.locales)) {
    if (locale === config.sourceLocale || !settings.published) continue;
    const state = localeState(locale, base);
    for (const unit of units.filter((unit) => unit.kind === "content")) {
      if (!state.manifest.content[unit.id]) continue;
      const [collection, ...rest] = unit.id.split("/");
      const to = path.join(target, collection, locale, ...rest);
      fs.mkdirSync(path.dirname(to), { recursive: true });
      const from = path.join(base, "locales", locale, "content", unit.id);
      if (unit.id.endsWith(".json")) {
        const paths = new Set(Object.keys(state.manifest.content).filter(id => id.endsWith(".mdx")).map(id => "/" + id.replace(/\.mdx$/, "").replace(/\/index$/, "")));
        fs.writeFileSync(to, JSON.stringify(localizeNavigation(read(from), locale, paths), null, 2) + "\n");
      } else fs.copyFileSync(from, to);
    }
  }
}
if (process.argv[1] && path.resolve(process.argv[1]) === fileURLToPath(import.meta.url)) {
  const [command, ...args] = process.argv.slice(2);
  const option = (name) => args[args.indexOf(name) + 1];
  try {
    if (command === "export") {
      const output = option("--output");
      if (!args.includes("--locale") || !args.includes("--output") || !output) throw new Error("Usage: i18n.mjs export --locale zh-CN --output /tmp/job.json");
      fs.writeFileSync(output, JSON.stringify(exportJob(option("--locale")), null, 2) + "\n");
      console.log(`Translation job written to ${output}`);
    } else if (command === "import") {
      if (!args.includes("--input")) throw new Error("Usage: i18n.mjs import --input /tmp/result.json [--reviewed]");
      console.log(`Imported ${importJob(read(option("--input")), { reviewed: args.includes("--reviewed") })} units`);
    } else if (command === "check") {
      const issues = check(); if (issues.length) throw new Error(issues.join("\n")); console.log("Localization contracts passed");
    } else if (command === "prepare") { prepare(); console.log("Prepared published translations"); }
    else throw new Error("Expected export, import, check, or prepare");
  } catch (error) { console.error(error.message); process.exitCode = 1; }
}
