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
