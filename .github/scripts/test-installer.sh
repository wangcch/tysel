#!/usr/bin/env bash
set -euo pipefail

if [[ $# -ne 3 ]]; then
  echo "usage: $0 <version> <target> <release-output>" >&2
  exit 2
fi

version="$1"
target="$2"
release_output="$3"
archive="tysel-${version}-${target}.tar.gz"
metadata="${release_output}/release-asset-${target}.json"
[[ -f "${release_output}/${archive}" && -f "${release_output}/${archive}.sha256" && -f "$metadata" ]] || {
  echo "installer fixture requires a packaged release target" >&2
  exit 1
}

fixture="$(mktemp -d)"
server_pid=
cleanup() {
  if [[ -n "$server_pid" ]]; then
    kill "$server_pid" 2>/dev/null || true
    wait "$server_pid" 2>/dev/null || true
  fi
  rm -rf "$fixture"
}
trap cleanup EXIT

mkdir -p "${fixture}/fake-bin"
printf '%s\n' '#!/bin/sh' 'if [ "$1" = "-s" ]; then printf "%s\\n" "$TEST_UNAME_S"; else printf "%s\\n" "$TEST_UNAME_M"; fi' \
  > "${fixture}/fake-bin/uname"
chmod +x "${fixture}/fake-bin/uname"
for mapping in 'Linux x86_64 linux-x64' 'Linux aarch64 linux-arm64' \
  'Darwin x86_64 darwin-x64' 'Darwin arm64 darwin-arm64'; do
  set -- $mapping
  output="$(env PATH="${fixture}/fake-bin:$PATH" TEST_UNAME_S="$1" TEST_UNAME_M="$2" \
    sh install.sh --version "$version" --prefix "${fixture}/dry root" --dry-run)"
  grep -F "target:  $3" <<< "$output" >/dev/null
done
if sh install.sh --version "$version" --prefix relative --dry-run >/dev/null 2>&1; then
  echo "installer accepted a relative root" >&2
  exit 1
fi
if env -u HOME -u TYSEL_HOME sh install.sh --version "$version" --dry-run \
  >"${fixture}/missing-home.log" 2>&1; then
  echo "installer accepted a missing HOME without an explicit prefix" >&2
  exit 1
fi
grep -F 'HOME is not set' "${fixture}/missing-home.log" >/dev/null

release_dir="${fixture}/server/download/v${version}"
mkdir -p "$release_dir" "${fixture}/server/latest/download" "${fixture}/home"
cp "${release_output}/${archive}" "${release_output}/${archive}.sha256" "$release_dir/"
printf '%s\n' "$version" > "${fixture}/server/latest/download/stable-version"
printf '%064d\n' 7 > "${fixture}/fixture.key"
chmod 600 "${fixture}/fixture.key"
key_info="$(target/repro-1/release/tysel release key-info --key "${fixture}/fixture.key")"
now_unix="$(date +%s)"
issued_at_unix="$((now_unix - 5))"
expires_at_unix="$((issued_at_unix + 7776000))"
jq -n --argjson key "$key_info" --argjson issued "$issued_at_unix" --argjson expires "$expires_at_unix" \
  '{policy_version: 1, issued_at_unix: $issued, expires_at_unix: $expires,
    keys: [{key_id: $key.key_id, algorithm: $key.algorithm,
      public_key: $key.public_key, status: "active", valid_from_unix: $issued}]}' \
  > "$release_dir/trust.json"

jq -s \
  --arg version "$version" \
  --arg target "$target" \
  --arg archiveUrl "https://fixture.invalid/${archive}" \
  --arg sha256 "$(awk 'NR == 1 { print $1 }' "${release_output}/${archive}.sha256")" \
  --argjson byteSize "$(wc -c < "${release_output}/${archive}" | tr -d '[:space:]')" \
  '.[0] + {
    archiveUrl: $archiveUrl,
    byteSize: $byteSize,
    sha256: $sha256,
    signature: {algorithm: "ed25519", url: "https://fixture.invalid/archive.sig.json", keyId: ("ab" * 32)}
  } | {
    schemaVersion: 1,
    version: $version,
    sourceCommit: .buildInfo[0].sourceCommit,
    publishedAt: "2026-08-22T00:00:00Z",
    channel: "stable",
    minimumUpdaterVersion: "0.0.1",
    compatibility: {minimumTapVersion: 1, maximumTapVersion: 3, capabilityAbiVersion: "0.4.0", typesVersion: $version},
    requiredFeatures: ["atomic-bin-link", "build-info-v1", "ed25519-manifest"],
    optionalFeatures: {},
    assets: [.]
  }' "$metadata" > "$release_dir/release-manifest.json"
target/repro-1/release/tysel release sign-metadata "$release_dir/release-manifest.json" \
  --key "${fixture}/fixture.key" >/dev/null
target/repro-1/release/tysel release sign-metadata "$release_dir/trust.json" \
  --key "${fixture}/fixture.key" >/dev/null
cp "$release_dir/trust.json" "$release_dir/trust.json.sig.json" \
  "${fixture}/server/latest/download/"

port=$((20000 + ($$ % 20000)))
python3 -m http.server "$port" --bind 127.0.0.1 --directory "${fixture}/server" \
  > "${fixture}/server.log" 2>&1 &
server_pid=$!
for _ in {1..50}; do
  curl -fsS "http://127.0.0.1:${port}/latest/download/stable-version" >/dev/null 2>&1 && break
  sleep 0.1
done
curl -fsS "http://127.0.0.1:${port}/latest/download/stable-version" >/dev/null

install_root="${fixture}/home/root with spaces"
env HOME="${fixture}/home" SHELL=/bin/sh \
  TYSEL_DOWNLOAD_BASE="http://127.0.0.1:${port}" \
  sh install.sh --prefix "$install_root" --no-modify-path
"${install_root}/bin/tysel" --version | grep -Fx "tysel ${version}"
[[ "$(readlink "${install_root}/bin")" == "versions/v${version}/bin" ]]
jq -e --arg version "$version" --arg target "$target" \
  '.schemaVersion == 1 and .activeVersion == $version and .target == $target' \
  "${install_root}/state.json" >/dev/null
TYSEL_DOWNLOAD_BASE="http://127.0.0.1:${port}" \
  "${install_root}/bin/tysel" upgrade --check --version "$version" --json \
  | jq -e '.schemaVersion == 1 and .changed == false and .toVersion == $version' \
      --arg version "$version" >/dev/null

# A same-version trust refresh is an explicit, truthfully reported mutation.
refreshed_issued="$(date +%s)"
refreshed_expires="$((refreshed_issued + 7776000))"
jq -n --argjson key "$key_info" --argjson issued "$refreshed_issued" \
  --argjson expires "$refreshed_expires" --argjson validFrom "$issued_at_unix" \
  '{policy_version: 1, issued_at_unix: $issued, expires_at_unix: $expires,
    keys: [{key_id: $key.key_id, algorithm: $key.algorithm,
      public_key: $key.public_key, status: "active", valid_from_unix: $validFrom}]}' \
  > "${fixture}/server/latest/download/trust.json"
target/repro-1/release/tysel release sign-metadata \
  "${fixture}/server/latest/download/trust.json" --key "${fixture}/fixture.key" >/dev/null
TYSEL_DOWNLOAD_BASE="http://127.0.0.1:${port}" \
  "${install_root}/bin/tysel" upgrade --yes --version "$version" --json \
  | jq -e '.schemaVersion == 1 and .action == "trust-refresh"
      and .changed == true and .fromVersion == $version and .toVersion == $version' \
      --arg version "$version" >/dev/null
cmp "${fixture}/server/latest/download/trust.json" "${install_root}/trust.json"
golden_project="${fixture}/golden-project"
env -i HOME="${fixture}/home" TMPDIR="${TMPDIR:-/tmp}" \
  PATH="${install_root}/bin:/usr/bin:/bin" \
  tysel init "$golden_project"
(cd "$golden_project" && env -i HOME="${fixture}/home" TMPDIR="${TMPDIR:-/tmp}" \
  PATH="${install_root}/bin:/usr/bin:/bin" tysel check)
(cd "$golden_project" && env -i HOME="${fixture}/home" TMPDIR="${TMPDIR:-/tmp}" \
  PATH="${install_root}/bin:/usr/bin:/bin" tysel test)

# A repeat install is idempotent and leaves the same active release.
env HOME="${fixture}/home" SHELL=/bin/sh \
  TYSEL_DOWNLOAD_BASE="http://127.0.0.1:${port}" \
  sh install.sh --version "$version" --prefix "$install_root"
[[ "$(readlink "${install_root}/bin")" == "versions/v${version}/bin" ]]
[[ "$(grep -Fc '# >>> tysel managed PATH >>>' "${fixture}/home/.profile")" == 1 ]]

# Reinstalling under a new root updates the existing managed PATH block.
alternate_root="${fixture}/home/alternate root"
env HOME="${fixture}/home" SHELL=/bin/sh \
  TYSEL_DOWNLOAD_BASE="http://127.0.0.1:${port}" \
  sh install.sh --version "$version" --prefix "$alternate_root"
grep -F "export PATH=\"${alternate_root}/bin:\$PATH\"" "${fixture}/home/.profile" >/dev/null
if grep -F "export PATH=\"${install_root}/bin:\$PATH\"" "${fixture}/home/.profile" >/dev/null; then
  echo "installer left the previous managed root in PATH" >&2
  exit 1
fi

# Bootstrap corruption must fail before activation and preserve the working link/state.
cp "${install_root}/state.json" "${fixture}/state.before"
printf '%064d\n' 0 > "${release_dir}/${archive}.sha256"
if env HOME="${fixture}/home" SHELL=/bin/sh \
  TYSEL_DOWNLOAD_BASE="http://127.0.0.1:${port}" \
  sh install.sh --version "$version" --prefix "$install_root" --no-modify-path; then
  echo "installer accepted a corrupt checksum" >&2
  exit 1
fi
cmp "${fixture}/state.before" "${install_root}/state.json"
[[ "$(readlink "${install_root}/bin")" == "versions/v${version}/bin" ]]
"${install_root}/bin/tysel" --version | grep -Fx "tysel ${version}"

# A correctly checksummed traversal archive is rejected by member policy.
python3 -c 'import io,sys,tarfile
p,n=sys.argv[1:]
with tarfile.open(p,"w:gz") as archive:
    entry=tarfile.TarInfo(n + "/share/acceptance/../../escape")
    entry.size=4
    archive.addfile(entry,io.BytesIO(b"nope"))' \
  "${release_dir}/${archive}" "tysel-${version}-${target}"
shasum -a 256 "${release_dir}/${archive}" | awk '{print $1}' \
  > "${release_dir}/${archive}.sha256"
if env HOME="${fixture}/home" SHELL=/bin/sh \
  TYSEL_DOWNLOAD_BASE="http://127.0.0.1:${port}" \
  sh install.sh --version "$version" --prefix "$install_root" --no-modify-path; then
  echo "installer accepted an archive traversal" >&2
  exit 1
fi
cmp "${fixture}/state.before" "${install_root}/state.json"
[[ "$(readlink "${install_root}/bin")" == "versions/v${version}/bin" ]]

echo "installer fixture passed for ${target}"
