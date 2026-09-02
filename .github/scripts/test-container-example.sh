#!/usr/bin/env bash
set -euo pipefail

repo_root="$(cd "$(dirname "${BASH_SOURCE[0]}")/../.." && pwd)"
version="${TYSEL_CONTAINER_TEST_VERSION:-$(bash "${repo_root}/.github/scripts/check-version-sync.sh")}"
toolchain_binary_directory="${TYSEL_TOOLCHAIN_BIN_DIR:-}"

image="tysel/hello-service:docs-test"
toolchain_image="tysel/toolchain:docs-test"
container="tysel-hello-service-docs-test"
context="${repo_root}/target/toolchain-image-test"

cleanup() {
  docker rm --force "$container" >/dev/null 2>&1 || true
}
trap cleanup EXIT

if [[ -n "$toolchain_binary_directory" ]]; then
  source_commit="$(git -C "$repo_root" rev-parse HEAD)"
  source_date_epoch="$(git -C "$repo_root" log -1 --pretty=%ct)"
  bash "${repo_root}/.github/scripts/prepare-toolchain-image.sh" \
    "$toolchain_binary_directory" "$context" "$version" linux-x64 ""
  docker build \
    --file "${repo_root}/.github/docker/toolchain.Dockerfile" \
    --build-arg "TYSEL_VERSION=${version}" \
    --build-arg "TYSEL_SOURCE_COMMIT=${source_commit}" \
    --build-arg "SOURCE_DATE_EPOCH=${source_date_epoch}" \
    --tag "$toolchain_image" \
    "$context"
else
  toolchain_image="ghcr.io/wangcch/tysel-toolchain:${version}"
fi

docker build \
  --build-arg "TYSEL_VERSION=${version}" \
  --build-arg "TYSEL_TOOLCHAIN_IMAGE=${toolchain_image}" \
  --tag "$image" \
  "${repo_root}/examples/hello-service"

docker run --detach \
  --name "$container" \
  --publish 127.0.0.1:3000:3000 \
  "$image" >/dev/null

curl --fail --retry 20 --retry-all-errors --retry-delay 1 \
  http://127.0.0.1:3000/healthz >/dev/null

echo "hello-service container example passed"
