#!/usr/bin/env bash
set -euo pipefail

root="$(cd "$(dirname "${BASH_SOURCE[0]}")/../.." && pwd)"
output="${1:-${root}/website/out}"
endpoint="oss-cn-hongkong.aliyuncs.com"

for variable in ALIYUN_OSS_ACCESS_KEY_ID ALIYUN_OSS_ACCESS_KEY_SECRET ALIYUN_OSS_BUCKET; do
  if [[ -z "${!variable:-}" ]]; then
    echo "${variable} is required" >&2
    exit 1
  fi
done

if [[ ! "$ALIYUN_OSS_BUCKET" =~ ^[a-z0-9][a-z0-9-]{1,62}$ ]]; then
  echo "ALIYUN_OSS_BUCKET is not a valid OSS bucket name" >&2
  exit 1
fi
if [[ ! -d "$output" || ! -s "$output/index.html" || ! -s "$output/404.html" ]]; then
  echo "static website output is missing index.html or 404.html: ${output}" >&2
  exit 1
fi
if [[ ! -s "$output/api/search" ]] || ! jq -e '.type == "advanced"' "$output/api/search" >/dev/null; then
  echo "static search index is missing or invalid" >&2
  exit 1
fi

config="$(mktemp)"
trap 'rm -f "$config"' EXIT
ossutil config -c "$config" -e "$endpoint" \
  -i "$ALIYUN_OSS_ACCESS_KEY_ID" -k "$ALIYUN_OSS_ACCESS_KEY_SECRET" -L EN

destination="oss://${ALIYUN_OSS_BUCKET}/"

# Keep HTML and route payloads fresh. Hashed Next.js assets are made immutable below.
ossutil sync "${output}/" "$destination" --delete --force \
  --meta "Cache-Control:public,max-age=300,must-revalidate" -c "$config"

# OSS does not compress static website responses on the fly. Replace text objects
# with deterministic gzip payloads while keeping their original object names.
while IFS= read -r -d '' file; do
  gzip --best --no-name --stdout "$file" > "${file}.gz"
  mv "${file}.gz" "$file"
done < <(
  find "$output" -type f \( \
    -name '*.css' -o -name '*.html' -o -name '*.js' -o -name '*.json' -o \
    -name '*.md' -o -name '*.mdx' -o -name '*.svg' -o -name '*.txt' -o \
    -name '*.xml' \
  \) -print0
)

ossutil cp "${output}/" "$destination" --recursive --force \
  --exclude "*" \
  --include "*.css" --include "*.html" --include "*.js" \
  --include "*.json" --include "*.md" --include "*.mdx" \
  --include "*.svg" --include "*.txt" --include "*.xml" \
  --meta "Content-Encoding:gzip#Cache-Control:public,max-age=300,must-revalidate" \
  -c "$config"

# The static search endpoint intentionally has no file extension.
gzip --best --no-name --stdout "$output/api/search" > "$output/api/search.gz"
mv "$output/api/search.gz" "$output/api/search"
ossutil cp "$output/api/search" "${destination}api/search" --force \
  --meta "Content-Type:application/json#Content-Encoding:gzip#Cache-Control:public,max-age=300,must-revalidate" \
  -c "$config"

ossutil set-meta "${destination}_next/static/" \
  "Cache-Control:public,max-age=31536000,immutable" \
  --recursive --force --update -c "$config"
for social_image in opengraph-image twitter-image; do
  ossutil set-meta "${destination}${social_image}" \
    "Content-Type:image/png#Cache-Control:public,max-age=300,must-revalidate" \
    --force --update -c "$config"
done

# Keep the index/error behavior and supported redirects in source control.
ossutil website --method put "oss://${ALIYUN_OSS_BUCKET}" \
  "${root}/website/oss/website.xml" -c "$config"

ossutil stat "${destination}index.html" -c "$config" >/dev/null
printf 'deployed %s to %s\n' "${GITHUB_SHA:-local}" "$destination"
