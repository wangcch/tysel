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

write_payload_hashes() {
  local root="$1"
  local output="$2"
  local file relative

  : > "$output"
  while IFS= read -r file; do
    relative="${file#"${root}/"}"
    printf '%s  %s\n' "$(sha256_file "$file")" "$relative" >> "$output"
  done < <(find "$root" -type f -print | LC_ALL=C sort)
}

rm -rf target/release-output
mkdir -p target/release-output
bash .github/scripts/reproducible-release.sh 1 "$version" "$release_target" "$source_date_epoch"
bash .github/scripts/reproducible-release.sh 2 "$version" "$release_target" "$source_date_epoch"
first_archive="target/${archive%.gz}.1.tar.gz"
second_archive="target/${archive%.gz}.2.tar.gz"
if [[ "$(sha256_file "$first_archive")" != "$(sha256_file "$second_archive")" ]]; then
  first_payload_hashes="target/release-payload-1.sha256"
  second_payload_hashes="target/release-payload-2.sha256"
  write_payload_hashes "target/archive-1/tysel-${version}-${release_target}" "$first_payload_hashes"
  write_payload_hashes "target/archive-2/tysel-${version}-${release_target}" "$second_payload_hashes"
  echo "release archive payload differences:" >&2
  if cmp -s "$first_payload_hashes" "$second_payload_hashes"; then
    echo "  file payloads are identical; the difference is in archive metadata" >&2
  else
    diff -u "$first_payload_hashes" "$second_payload_hashes" >&2 || true
  fi
fi
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
