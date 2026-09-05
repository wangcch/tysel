# Tysel website

Public site for [tysel.dev](https://tysel.dev): marketing pages plus documentation.

```sh
cd website
pnpm install
pnpm dev
```

Open [http://localhost:4000](http://localhost:4000).

## Production deployment

`pnpm build` exports the complete site to `out/`. Pull requests build and verify
the export; pushes to `main` that change the website, docs, or brand deploy it to
the Hong Kong OSS bucket through `.github/workflows/website.yml`. Text assets are
precompressed because this deployment intentionally does not require a CDN.

Create the `website-production` GitHub Environment with:

- secret `ALIYUN_OSS_ACCESS_KEY_ID`
- secret `ALIYUN_OSS_ACCESS_KEY_SECRET`
- variable `ALIYUN_OSS_BUCKET` containing only the bucket name

The RAM identity needs object list/read/write/delete permissions for that bucket
and `oss:PutBucketWebsite`. Enable bucket versioning before the first deployment:
deployment synchronizes with `--delete`, so versioning is the recovery path for
an accidental or incomplete upload.

Configure the bucket as public-read, bind `tysel.dev`, and install its TLS
certificate in OSS. In Cloudflare, point the apex CNAME at the Hong Kong bucket
endpoint with proxying disabled (DNS only). The checked-in OSS website
configuration serves directory indexes and redirects `/install.sh` to the latest
GitHub Release.

| Path | Source |
| --- | --- |
| `/` | Product homepage (bun.sh-density layout) |
| `/blog` | Product blog (MDX under `content/blog/`) |
| `/docs` | Migrated from `docs/` |
| `/examples` | Example gallery |
| `/benchmarks` | Admission results from CI evidence (checked-in default is unpublished) |
| `/rss.xml` | Blog RSS feed |

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

Sync the latest fresh, successful **main-branch push**
`benchmark-evidence-linux-x64` artifact from the canonical CI workflow into
`data/benchmarks/admission-linux-x64.json` (requires `GH_TOKEN`):

```sh
pnpm sync:benchmarks
```

On main, website CI starts only after the canonical CI workflow succeeds. It
fetches that exact workflow run before `pnpm build`, validates all seven gates,
binds both automatic and manual deployments to the checked-out source commit,
and rechecks every deployment source SHA against the current main head
immediately before publishing. Superseded automatic or manual runs cannot
overwrite a newer website. PRs build against the checked-in unpublished
snapshot and never fetch or deploy benchmark evidence. The checked-in default is
`{"status":"unpublished"}` so local builds without sync show empty measured
cells. Missing, older-than-30-days, mismatched, or invalid artifacts also write
`unpublished` (build continues; stale published numbers are not kept). The exact
build-time envelope is exported at `/benchmark-evidence/latest.json`. For a
safe layout-only preview using the marked sample fixture (without replacing the
checked-in default), run:

```sh
TYSEL_BENCHMARK_SAMPLE=1 pnpm dev
```

The homepage follows bun.sh’s information density and developer-tool layout, using Tysel brand color, the official wordmark, and product copy from the docs. It does not publish unverified performance numbers.

## Localization

The website has build-time localization. English remains the source language and
is published at unprefixed URLs. Simplified Chinese has a complete reviewed UI and
content release set; publication is controlled by `locales/config.json`.
Development and builds never call a translation model or service.

After changing canonical files under the repository-level `docs/` directory,
run `pnpm import:docs` before exporting translation work. Source changes invalidate
the corresponding reviewed translation by hash; update and review that unit again
instead of editing generated content. Run `pnpm i18n:check` before submitting any
UI or documentation copy change.

UI messages live in `locales/<locale>/messages.json`; document translations live
in `locales/<locale>/content/{docs,reference,blog}/`. Keep UI keys stable and use
`T` or `useLocale().t()` for new UI copy. Follow the locale's `glossary.json`.
`manifest.json` records source and translation hashes plus review status.

Translate outside the build using the model of your choice:

```sh
pnpm i18n:export --locale zh-CN --output /tmp/tysel-zh-CN-job.json
# Return job units with a translation string; partial results are accepted.
pnpm i18n:import --input /tmp/tysel-zh-CN-result.json
# Review wording and technical meaning before marking the result reviewed.
pnpm i18n:import --input /tmp/tysel-zh-CN-result.json --reviewed
pnpm i18n:check
```

Imports default to draft. Preserve code blocks, URLs, placeholders, metadata,
navigation IDs/order and heading counts/depths. Review technical qualifications,
terminology and paragraph logic; blog translations should read naturally while
preserving historical version context. Editing reviewed text requires another
review. Published stale or draft translations fail the build.

To add a language, register its BCP-47 code with `published: false`, create its
message catalog and version-1 manifest (empty `messages` and `content` maps), then
export, translate and review. Enable publication after all UI messages and the
included documents are reviewed and current. Documents can be added gradually.

English URLs remain unprefixed; Chinese uses `/zh-CN/`. Only existing translations
receive localized URLs and language alternates. Links to untranslated pages fall
back to English. `lib/i18n/pages.ts` controls availability, and each published
language has its own search data. Never edit generated `app/(localized)` adapters
or `content/translations/` content; preparation recreates them. Translated headings retain
English anchor aliases.

After building, verify the exported pages:

```sh
python3 scripts/check-localized-html.py
python3 scripts/check-seo.py
```

CI runs both checks for section links, document roots, blog titles, sitemap pages,
canonical URLs, reciprocal language alternates and structured data. These local
checks do not establish search indexing or real-world performance; verify those
after deployment with Search Console and performance measurements.
