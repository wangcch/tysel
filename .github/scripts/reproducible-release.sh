#!/usr/bin/env bash
set -euo pipefail

if [[ $# -ne 4 ]]; then
  echo "usage: $0 <ordinal> <version> <target> <source-date-epoch>" >&2
  exit 2
fi

ordinal="$1"
version="$2"
release_target="$3"
source_date_epoch="$4"

[[ "$ordinal" =~ ^[12]$ ]] || { echo "ordinal must be 1 or 2" >&2; exit 2; }
[[ "$version" =~ ^[0-9]+\.[0-9]+\.[0-9]+([+-][0-9A-Za-z.-]+)?$ ]] || {
  echo "release version must be canonical semver without a v prefix" >&2
  exit 2
}
[[ "$release_target" =~ ^[a-z0-9-]+$ ]] || { echo "invalid release target" >&2; exit 2; }
[[ "$source_date_epoch" =~ ^[0-9]+$ ]] || { echo "invalid SOURCE_DATE_EPOCH" >&2; exit 2; }

export SOURCE_DATE_EPOCH="$source_date_epoch"
export CARGO_TARGET_DIR="${PWD}/target/repro-${ordinal}"
# rustc applies the last matching remap, so the more specific target path must
# follow the workspace path.
path_remap="--remap-path-prefix=${PWD}=/src --remap-path-prefix=${CARGO_TARGET_DIR}=/build"
case "$release_target" in
  linux-x64|linux-arm64)
    export RUSTFLAGS="${path_remap} -C link-arg=-Wl,--build-id=none"
    ;;
  darwin-x64|darwin-arm64)
    export RUSTFLAGS="$path_remap"
    export COPYFILE_DISABLE=1
    ;;
  *)
    echo "unsupported release target ${release_target}" >&2
    exit 2
    ;;
esac
export TYSEL_SOURCE_COMMIT="${TYSEL_SOURCE_COMMIT:-$(git rev-parse HEAD)}"
export TYSEL_RELEASE_ID="$version"

[[ "$TYSEL_SOURCE_COMMIT" =~ ^[0-9a-f]{40}$ ]] || {
  echo "TYSEL_SOURCE_COMMIT must be a canonical commit" >&2
  exit 2
}

archive="tysel-${version}-${release_target}.tar.gz"
root="target/archive-${ordinal}/tysel-${version}-${release_target}"
acceptance="${root}/share/acceptance"
release_app="target/release-${ordinal}/hello-service"
build_info="target/release-build-info-${ordinal}"
asset_metadata="target/release-asset-${ordinal}.json"
archive_list="${PWD}/target/release-archive-${ordinal}.list"

sha256_file() {
  if command -v sha256sum > /dev/null 2>&1; then
    sha256sum "$1" | awk '{print $1}'
  else
    shasum -a 256 "$1" | awk '{print $1}'
  fi
}

cargo build --locked --release -p tysel-cli -p tysel-runtime -p tysel-isolate
rm -rf "target/archive-${ordinal}" "target/release-${ordinal}" "$build_info"
rm -f "target/${archive%.gz}.${ordinal}.tar" "target/${archive%.gz}.${ordinal}.tar.gz"
mkdir -p "${root}/bin" "$acceptance" "target/release-${ordinal}"
install -m 755 "${CARGO_TARGET_DIR}/release/tysel" "${root}/bin/tysel"
install -m 755 "${CARGO_TARGET_DIR}/release/tysel-service" "${root}/bin/tysel-service"
install -m 755 "${CARGO_TARGET_DIR}/release/tysel-worker" "${root}/bin/tysel-worker"
install -m 644 LICENSE README.md "$root"
mkdir -p "$build_info"
for binary in tysel tysel-service tysel-worker; do
  "${root}/bin/${binary}" --build-info-json > "${build_info}/${binary}.json"
done
jq -s -e \
  --arg target "$release_target" \
  --arg sourceCommit "$TYSEL_SOURCE_COMMIT" \
  --arg releaseId "$TYSEL_RELEASE_ID" \
  'length == 3
    and all(.[]; .schemaVersion == 1)
    and ([.[].binary] | sort) == ["tysel", "tysel-service", "tysel-worker"]
    and all(.[]; .version == $releaseId)
    and all(.[]; .target == $target)
    and all(.[]; .sourceCommit == $sourceCommit)
    and all(.[]; .releaseId == $releaseId)' \
  "${build_info}"/*.json > /dev/null
case "$release_target" in
  linux-x64|linux-arm64)
    platform='{"minimumGlibc":"2.39","minimumKernel":"6.8"}'
    ;;
  darwin-x64|darwin-arm64)
    platform='{"minimumMacos":"13.0"}'
    ;;
  *)
    echo "unsupported release target ${release_target}" >&2
    exit 2
    ;;
esac
jq -s \
  --arg target "$release_target" \
  --arg tyselSha256 "$(sha256_file "${root}/bin/tysel")" \
  --arg serviceSha256 "$(sha256_file "${root}/bin/tysel-service")" \
  --arg workerSha256 "$(sha256_file "${root}/bin/tysel-worker")" \
  --argjson platform "$platform" \
  '{target: $target,
    files: [
      {path: "bin/tysel", sha256: $tyselSha256},
      {path: "bin/tysel-service", sha256: $serviceSha256},
      {path: "bin/tysel-worker", sha256: $workerSha256}
    ],
    buildInfo: sort_by(.binary),
    platform: $platform}' \
  "${build_info}"/*.json > "$asset_metadata"
"${CARGO_TARGET_DIR}/release/tysel" build \
  --manifest examples/hello-service/tysel.toml \
  --stub "${CARGO_TARGET_DIR}/release/tysel-service" \
  --output "$release_app" \
  --release
cp "${release_app}"* "$acceptance"
if [[ "$OSTYPE" == darwin* ]]; then
  archive_stamp="$(date -u -r "$SOURCE_DATE_EPOCH" +%Y%m%d%H%M.%S)"
  find "$root" -exec touch -t "$archive_stamp" {} +
else
  find "$root" -exec touch -d "@${SOURCE_DATE_EPOCH}" {} +
fi
if tar --version 2>&1 | grep -q 'GNU tar'; then
  tar --sort=name --format=ustar --owner=0 --group=0 --numeric-owner \
    -C "target/archive-${ordinal}" -cf "target/${archive%.gz}.${ordinal}.tar" \
    "tysel-${version}-${release_target}"
else
  (cd "target/archive-${ordinal}" \
    && find "tysel-${version}-${release_target}" -print | LC_ALL=C sort) > "$archive_list"
  tar --no-recursion --format=ustar --uid 0 --gid 0 --numeric-owner \
    -C "target/archive-${ordinal}" -cf "target/${archive%.gz}.${ordinal}.tar" \
    -T "$archive_list"
fi
gzip -n -9 "target/${archive%.gz}.${ordinal}.tar"

archive_check="target/release-archive-check-${ordinal}"
rm -rf "$archive_check"
mkdir -p "$archive_check"
tar -xzf "target/${archive%.gz}.${ordinal}.tar.gz" -C "$archive_check"
"${archive_check}/tysel-${version}-${release_target}/bin/tysel" doctor --install --json \
  | jq -e '.schemaVersion == 1 and .summary.failed == 0' > /dev/null
