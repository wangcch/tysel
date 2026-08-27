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
| `/docs` | Migrated from `docs/` |
| `/examples` | Example gallery |
| `/benchmarks` | Admission results from CI evidence (checked-in default is unpublished) |

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
