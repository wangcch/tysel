#!/usr/bin/env bash
set -euo pipefail

if [[ $# -ne 2 ]]; then
  echo "usage: $0 <target> <source-date-epoch>" >&2
  exit 2
fi

release_target="$1"
source_date_epoch="$2"
version="$(bash .github/scripts/check-version-sync.sh)"
archive="tysel-${version}-${release_target}.tar.gz"

sha256_file() {
  if command -v sha256sum > /dev/null 2>&1; then
    sha256sum "$1" | awk '{print $1}'
  else
    shasum -a 256 "$1" | awk '{print $1}'
  fi
}

rm -rf target/release-output
mkdir -p target/release-output
bash .github/scripts/reproducible-release.sh 1 "$version" "$release_target" "$source_date_epoch"
bash .github/scripts/reproducible-release.sh 2 "$version" "$release_target" "$source_date_epoch"
mv "target/${archive%.gz}.1.tar.gz" "target/release-output/${archive}"
mv target/release-asset-1.json "target/release-output/release-asset-${release_target}.json"
mv "target/${archive%.gz}.2.tar.gz" "target/${archive}.second"
first_command="bash .github/scripts/reproducible-release.sh 1 ${version} ${release_target} ${source_date_epoch}"
second_command="bash .github/scripts/reproducible-release.sh 2 ${version} ${release_target} ${source_date_epoch}"
target/repro-1/release/tysel release reproduce \
  "target/release-output/${archive}" "target/${archive}.second" \
  --source-commit "$GITHUB_SHA" \
  --target "$release_target" \
  --toolchain "$(rustc --version)" \
  --lockfile Cargo.lock \
  --command "$first_command" \
  --command "$second_command" \
  --output "target/release-output/${archive}.repro.json"
target/repro-1/release/tysel release verify-reproducibility \
  "target/release-output/${archive}" \
  --evidence "target/release-output/${archive}.repro.json" \
  --lockfile Cargo.lock \
  --target "$release_target"
sha256_file "target/release-output/${archive}" > "target/release-output/${archive}.sha256"
