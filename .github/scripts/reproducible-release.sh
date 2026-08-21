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
[[ "$version" =~ ^[A-Za-z0-9._-]+$ ]] || { echo "invalid release version" >&2; exit 2; }
[[ "$release_target" =~ ^[a-z0-9-]+$ ]] || { echo "invalid release target" >&2; exit 2; }
[[ "$source_date_epoch" =~ ^[0-9]+$ ]] || { echo "invalid SOURCE_DATE_EPOCH" >&2; exit 2; }

export SOURCE_DATE_EPOCH="$source_date_epoch"
export CARGO_TARGET_DIR="target/repro-${ordinal}"
export RUSTFLAGS="--remap-path-prefix=${PWD}=/src -C link-arg=-Wl,--build-id=none"

archive="tysel-${version}-${release_target}.tar.gz"
root="target/archive-${ordinal}/tysel-${version}-${release_target}"
acceptance="${root}/share/acceptance"
release_app="target/release-${ordinal}/hello-service"

cargo build --locked --release -p tysel-cli -p tysel-runtime -p tysel-isolate
rm -rf "target/archive-${ordinal}" "target/release-${ordinal}"
rm -f "target/${archive%.gz}.${ordinal}.tar" "target/${archive%.gz}.${ordinal}.tar.gz"
mkdir -p "${root}/bin" "$acceptance" "target/release-${ordinal}"
install -m 755 "${CARGO_TARGET_DIR}/release/tysel" "${root}/bin/tysel"
install -m 755 "${CARGO_TARGET_DIR}/release/tysel-service" "${root}/bin/tysel-service"
install -m 755 "${CARGO_TARGET_DIR}/release/tysel-worker" "${root}/bin/tysel-worker"
install -m 644 LICENSE README.md "$root"
"${CARGO_TARGET_DIR}/release/tysel" build \
  --manifest examples/hello-service/tysel.toml \
  --stub "${CARGO_TARGET_DIR}/release/tysel-service" \
  --output "$release_app" \
  --release
cp "${release_app}"* "$acceptance"
tar --sort=name --mtime="@${SOURCE_DATE_EPOCH}" --owner=0 --group=0 --numeric-owner \
  -C "target/archive-${ordinal}" -cf "target/${archive%.gz}.${ordinal}.tar" \
  "tysel-${version}-${release_target}"
gzip -n -9 "target/${archive%.gz}.${ordinal}.tar"
