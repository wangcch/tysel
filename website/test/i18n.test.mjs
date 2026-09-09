import test from "node:test";
import assert from "node:assert/strict";
import fs from "node:fs";
import os from "node:os";
import path from "node:path";
import { hash, sourceUnits, exportJob, importJob, check, prepare } from "../scripts/i18n.mjs";

function fixture(t) {
  const base = fs.mkdtempSync(path.join(os.tmpdir(), "tysel-i18n-"));
  t.after(() => fs.rmSync(base, { recursive: true, force: true }));
  for (const dir of ["locales/en", "locales/zh-CN", "content/docs"]) fs.mkdirSync(path.join(base, dir), { recursive: true });
  const write = (file, value) => fs.writeFileSync(path.join(base, file), typeof value === "string" ? value : JSON.stringify(value));
  write("locales/config.json", { sourceLocale: "en", locales: { en: { published: true }, "zh-CN": { published: false } } });
  write("locales/en/messages.json", { "nav.home": "Home", "greeting": "Hello {name}" });
  write("locales/zh-CN/messages.json", {});
  write("locales/zh-CN/manifest.json", { version: 1, messages: {}, content: {} });
  write("content/docs/index.mdx", '---\ntitle: Documentation\n---\nRead [install](/docs/install).\n\n```sh\ntysel doctor\n```\n');
  return { base, write, publish() { write("locales/config.json", { sourceLocale: "en", locales: { en: { published: true }, "zh-CN": { published: true } } }); } };
}
function result(base) {
  const job = exportJob("zh-CN", base);
  // Synthetic fixture output, never repository translations or a model call.
  return { ...job, units: job.units.map((unit) => ({ ...unit, translation: unit.source })) };
}
test("draft locale can be configured with zero translations", (t) => {
  const { base } = fixture(t);
  assert.deepEqual(check(base), []);
  prepare(base);
  assert.deepEqual(fs.readdirSync(path.join(base, "content/translations/docs")), []);
  assert.equal(exportJob("zh-CN", base).units.length, 3);
});
test("publication requires reviewed, current UI messages", (t) => {
  const f = fixture(t); f.publish();
  assert.equal(check(f.base).length, 2);
  importJob(result(f.base), { base: f.base });
  assert.match(check(f.base).join("\n"), /draft/);
});
test("reviewed partial content prepares separately and English stays intact", (t) => {
  const f = fixture(t), before = fs.readFileSync(path.join(f.base, "content/docs/index.mdx"), "utf8");
  importJob(result(f.base), { base: f.base, reviewed: true }); f.publish();
  assert.deepEqual(check(f.base), []); prepare(f.base);
  assert.equal(fs.readFileSync(path.join(f.base, "content/translations/docs/zh-CN/index.mdx"), "utf8"), before);
  assert.equal(fs.readFileSync(path.join(f.base, "content/docs/index.mdx"), "utf8"), before);
  assert.equal(exportJob("zh-CN", f.base).units.length, 0);
});
test("incremental export detects source edits and blocks stale publication", (t) => {
  const f = fixture(t); importJob(result(f.base), { base: f.base, reviewed: true }); f.publish();
  f.write("locales/en/messages.json", { "nav.home": "Homepage", greeting: "Hello {name}" });
  assert.equal(exportJob("zh-CN", f.base).units.length, 1);
  assert.match(check(f.base).join("\n"), /nav.home.*stale/);
});
test("stale, duplicate, and unknown/path traversal units are rejected before writes", (t) => {
  const { base } = fixture(t), job = result(base);
  for (const units of [[{ ...job.units[0], sourceHash: "old" }], [job.units[0], job.units[0]], [{ ...job.units[0], kind: "content", id: "../../outside.mdx" }]]) {
    assert.throws(() => importJob({ ...job, units }, { base }));
    assert.deepEqual(JSON.parse(fs.readFileSync(path.join(base, "locales/zh-CN/messages.json"))), {});
  }
});
test("placeholders, code blocks, and link destinations cannot silently change", (t) => {
  const { base } = fixture(t), job = result(base);
  for (const [find, replace] of [["{name}", "{user}"], ["tysel doctor", "other command"], ["/docs/install", "/other"]]) {
    const units = job.units.map((unit) => ({ ...unit, translation: unit.translation.replace(find, replace) }));
    assert.throws(() => importJob({ ...job, units }, { base }));
  }
});
test("publishing a locale does not require all documents to be translated", (t) => {
  const f = fixture(t), job = result(f.base);
  importJob({ ...job, units: job.units.filter((unit) => unit.kind === "message") }, { base: f.base, reviewed: true });
  f.publish(); assert.deepEqual(check(f.base), []); prepare(f.base);
  assert.deepEqual(fs.readdirSync(path.join(f.base, "content/translations/docs")), []);
});
test("removed locale content cannot survive the preparation step", (t) => {
  const f = fixture(t); importJob(result(f.base), { base: f.base, reviewed: true }); f.publish(); prepare(f.base);
  f.write("locales/config.json", { sourceLocale: "en", locales: { en: { published: true }, "zh-CN": { published: false } } });
  prepare(f.base); assert.deepEqual(fs.readdirSync(path.join(f.base, "content/translations/docs")), []);
});
test("source fingerprint hashes exact UTF-8 content", (t) => {
  const { base } = fixture(t);
  for (const unit of sourceUnits(base)) assert.equal(unit.sourceHash, hash(unit.source));
});
test("editing a reviewed translation requires another review", (t) => {
  const f = fixture(t); importJob(result(f.base), { base: f.base, reviewed: true }); f.publish();
  f.write("locales/zh-CN/messages.json", { "nav.home": "Modified", greeting: "Hello {name}" });
  assert.match(check(f.base).join("\n"), /changed since review/);
});
test("locale route adapters exist only for published target locales", (t) => {
  const f = fixture(t); prepare(f.base); assert.equal(fs.existsSync(path.join(f.base, "app/(localized)")), false);
  importJob(result(f.base), { base: f.base, reviewed: true }); f.publish(); prepare(f.base);
  assert.equal(fs.existsSync(path.join(f.base, "app/(localized)/[lang]/[[...path]]/page.tsx")), true);
});
