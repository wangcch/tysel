#!/usr/bin/env bash
set -euo pipefail

if [[ $# -ne 5 ]]; then
  echo "usage: $0 <binary-directory> <context-directory> <version> <target> <source-commit-or-empty>" >&2
  exit 2
fi

binary_directory="$1"
context_directory="$2"
version="$3"
expected_target="$4"
expected_source_commit="$5"
repo_root="$(cd "$(dirname "${BASH_SOURCE[0]}")/../.." && pwd)"

[[ -d "$binary_directory" ]] || {
  echo "binary directory does not exist: $binary_directory" >&2
  exit 1
}
[[ "$version" =~ ^[0-9]+\.[0-9]+\.[0-9]+([+-][0-9A-Za-z.-]+)*$ ]] || {
  echo "invalid Tysel version: $version" >&2
  exit 2
}
[[ "$expected_target" =~ ^linux-(x64|arm64)$ ]] || {
  echo "toolchain images support linux-x64 and linux-arm64" >&2
  exit 2
}
if [[ -n "$expected_source_commit" && ! "$expected_source_commit" =~ ^[0-9a-f]{40}$ ]]; then
  echo "source commit must be empty or a canonical commit" >&2
  exit 2
fi

for binary in tysel tysel-service tysel-worker; do
  path="${binary_directory}/${binary}"
  [[ -x "$path" ]] || {
    echo "missing executable: $path" >&2
    exit 1
  }
  build_info="$($path --build-info-json)"
  jq -e \
    --arg binary "$binary" \
    --arg version "$version" \
    --arg target "$expected_target" \
    --arg sourceCommit "$expected_source_commit" \
    '.schemaVersion == 1
      and .binary == $binary
      and .version == $version
      and .target == $target
      and ($sourceCommit == "" or .sourceCommit == $sourceCommit)' \
    <<< "$build_info" > /dev/null || {
      echo "unexpected build information for $path" >&2
      exit 1
    }
done

[[ ! -L "$context_directory" ]] || {
  echo "context directory must not be a symbolic link: $context_directory" >&2
  exit 2
}
mkdir -p "$context_directory"
context_directory="$(cd "$context_directory" && pwd -P)"
case "$context_directory" in
  "${repo_root}/target/"*) ;;
  *)
    echo "context directory must be below ${repo_root}/target" >&2
    exit 2
    ;;
esac
rm -rf "${context_directory}/bin"
rm -f "${context_directory}/LICENSE"
mkdir -p "${context_directory}/bin"
for binary in tysel tysel-service tysel-worker; do
  install -m 755 "${binary_directory}/${binary}" "${context_directory}/bin/${binary}"
done
license_source="${repo_root}/LICENSE"
if [[ -f "${binary_directory}/../LICENSE" ]]; then
  license_source="${binary_directory}/../LICENSE"
fi
install -m 644 "$license_source" "${context_directory}/LICENSE"

echo "prepared ${expected_target} toolchain image context at ${context_directory}"
