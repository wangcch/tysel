import fs from "node:fs";
import path from "node:path";

const repoRoot = path.resolve(import.meta.dirname, "../..");
const brandLogo = path.join(repoRoot, "brand/logo");
const outDir = path.resolve(import.meta.dirname, "../public/brand");

const files = ["tysel-wordmark.svg", "tysel-mark.svg"];

fs.mkdirSync(outDir, { recursive: true });

for (const file of files) {
  const src = path.join(brandLogo, file);
  const dest = path.join(outDir, file);
  fs.copyFileSync(src, dest);
  console.log(`synced ${path.relative(repoRoot, src)} → ${path.relative(repoRoot, dest)}`);
}
