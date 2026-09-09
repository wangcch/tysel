#!/usr/bin/env bash
set -euo pipefail

if [[ $# -ne 1 ]]; then
  echo "usage: $0 <target>" >&2
  exit 2
fi

export CARGO_TARGET_DIR="${CARGO_TARGET_DIR:-${PWD}/target}"
if [[ "$CARGO_TARGET_DIR" != /* ]]; then
  export CARGO_TARGET_DIR="${PWD}/${CARGO_TARGET_DIR}"
fi
# Use the workspace release profile for both CI measurements and shipped files.
# The more specific remap must follow the workspace remap.
path_remap="--remap-path-prefix=${PWD}=/src --remap-path-prefix=${CARGO_TARGET_DIR}=/build"
case "$1" in
  linux-x64|linux-arm64)
    export RUSTFLAGS="${path_remap} -C link-arg=-Wl,--build-id=none"
    ;;
  darwin-x64|darwin-arm64)
    export RUSTFLAGS="$path_remap"
    ;;
  *)
    echo "unsupported release target $1" >&2
    exit 2
    ;;
esac

export TYSEL_SOURCE_COMMIT="${TYSEL_SOURCE_COMMIT:-$(git rev-parse HEAD)}"
export TYSEL_RELEASE_ID="${TYSEL_RELEASE_ID:-$(bash .github/scripts/check-version-sync.sh)}"

# Keep the package set identical: Cargo feature unification can change size.
cargo build --locked --release -p tysel-cli -p tysel-runtime -p tysel-isolate
