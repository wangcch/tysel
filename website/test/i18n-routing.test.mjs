import test from "node:test";
import assert from "node:assert/strict";
import fs from "node:fs";
import os from "node:os";
import path from "node:path";
import { pathToFileURL } from "node:url";
import ts from "typescript";

// Compile the actual TS helpers without Next.js or a browser runtime.
const dir = fs.mkdtempSync(path.join(os.tmpdir(), "tysel-routing-"));
const config = JSON.parse(fs.readFileSync(new URL("../locales/config.json", import.meta.url), "utf8"));
for (const name of ["config", "routing"]) {
  let source = fs.readFileSync(new URL(`../lib/i18n/${name}.ts`, import.meta.url), "utf8");
  source = source.replace('import config from "../../locales/config.json";', `const config = ${JSON.stringify(config)};`)
    .replace('from "./config"', 'from "./config.mjs"');
  fs.writeFileSync(path.join(dir, `${name}.mjs`), ts.transpileModule(source, { compilerOptions: { target: ts.ScriptTarget.ES2022, module: ts.ModuleKind.ESNext } }).outputText);
}
const { localePath, splitLocale, availableLocalePath } = await import(pathToFileURL(path.join(dir, "routing.mjs")).href);
process.on("exit", () => fs.rmSync(dir, { recursive: true, force: true }));
test("English URL migration preserves the old canonical paths", () => {
  assert.equal(localePath("/docs/install/", "en"), "/docs/install");
  assert.equal(localePath("/zh-CN/docs/install/?from=nav#step", "en"), "/docs/install?from=nav#step");
});
test("locale prefixing is idempotent and preserves query and fragment", () => {
  assert.equal(localePath("/", "zh-CN"), "/zh-CN");
  assert.equal(localePath("/zh-CN/docs/install/?q=a#b", "zh-CN"), "/zh-CN/docs/install?q=a#b");
  assert.deepEqual(splitLocale("/zh-CN/docs"), { locale: "zh-CN", pathname: "/docs" });
});
test("untranslated pages and assets link to original URLs, not invented locale routes", () => {
  assert.equal(availableLocalePath("/docs/install#step", "zh-CN", ["/", "/docs"]), "/docs/install#step");
  assert.equal(availableLocalePath("/docs", "zh-CN", ["/docs"]), "/zh-CN/docs");
  for (const path of ["/rss.xml", "/install.sh", "/benchmark-evidence/latest.json", "https://example.com/docs", "//example.com/docs", "#step", "mailto:a@example.com"]) {
    assert.equal(availableLocalePath(path, "zh-CN", ["/docs"]), path);
  }
});
