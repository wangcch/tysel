# Tysel website

Public site for [tysel.dev](https://tysel.dev): marketing pages plus documentation.

```sh
cd website
pnpm install
pnpm dev
```

Open [http://localhost:4000](http://localhost:4000).

| Path | Source |
| --- | --- |
| `/` | Product homepage (bun.sh-density layout) |
| `/docs` | Migrated from `docs/` |
| `/examples` | Example gallery |
| `/benchmarks` | Methodology and admission gates |

Brand logos sync from the repo root on `predev` / `prebuild`:

```sh
pnpm sync:brand
# brand/logo/tysel-wordmark.svg → public/brand/tysel-wordmark.svg
# brand/logo/tysel-mark.svg     → public/brand/tysel-mark.svg
```

Re-import documentation after `docs/` changes:

```sh
node scripts/import-docs.mjs
```

The homepage follows bun.sh’s information density and developer-tool layout, using Tysel brand color, the official wordmark, and product copy from the docs. It does not publish unverified performance numbers.
